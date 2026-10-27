//! Renders a stub library on every default theme screen, and in a child theme, to PNG files, with
//! Slint's software renderer and no window. Its own test binary: it installs its own Slint
//! platform.

use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use crate::common::*;
use nexus_core::{Area, Command, Route};
use slint::platform::software_renderer::{MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint_interpreter::ComponentHandle;

struct Offscreen(Rc<MinimalSoftwareWindow>);

thread_local! {
    /// Slint's time on this thread: it moves only when [`pass`] moves it.
    static ELAPSED: std::cell::Cell<Duration> = const { std::cell::Cell::new(Duration::ZERO) };
}

/// Moves Slint's time on by `by`, for its timers and animations.
fn pass(by: Duration) {
    ELAPSED.with(|e| e.set(e.get() + by));
}

/// Runs the core on its clock until `done`, for at most `ms` milliseconds of it.
fn run_until(host: &Composed, ms: u64, done: impl Fn(&nexus_core::Core) -> bool) {
    for _ in 0..ms / 10 {
        if done(host.core()) {
            return;
        }
        host.advance(Duration::from_millis(10));
    }
}

impl Platform for Offscreen {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
    fn duration_since_start(&self) -> Duration {
        ELAPSED.with(|e| e.get())
    }
    fn debug_log(&self, arguments: std::fmt::Arguments) {
        if keeps_log_line(&arguments) {
            eprintln!("{arguments}");
        }
    }
}

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 760;

fn shoot(window: &MinimalSoftwareWindow, path: &Path) {
    let size = window.size();
    let (width, height) = (size.width, size.height);
    let mut pixels = vec![PremultipliedRgbaColor::default(); (width * height) as usize];
    // A screen is made, and its entrance starts, at its first frame; draw that frame, let the
    // animations finish, then draw the one to keep.
    slint::platform::update_timers_and_animations();
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, width as usize);
    });
    // A timer that a change handler starts runs from the next pass: the hero's and the
    // art's wait for details takes three, as the event loop would make them.
    for _ in 0..2 {
        pass(Duration::from_millis(500));
        slint::platform::update_timers_and_animations();
    }
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, width as usize);
    });
    let bytes: Vec<u8> = pixels.iter().flat_map(|p| [p.red, p.green, p.blue, p.alpha]).collect();
    image::save_buffer(path, &bytes, width, height, image::ColorType::Rgba8).unwrap();
}

/// The next frame only, with no time for animations: a screen's first frame.
fn shoot_first(window: &MinimalSoftwareWindow, path: &Path) {
    let size = window.size();
    let mut pixels = vec![PremultipliedRgbaColor::default(); (size.width * size.height) as usize];
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, size.width as usize);
    });
    let bytes: Vec<u8> = pixels.iter().flat_map(|p| [p.red, p.green, p.blue, p.alpha]).collect();
    image::save_buffer(path, &bytes, size.width, size.height, image::ColorType::Rgba8).unwrap();
}

/// Runs the theme's change handlers, then carries out what they asked for: the download sheet
/// opens Downloads once the download is there.
fn follow(host: &mut Composed) {
    host.ui.render().unwrap();
    slint::platform::update_timers_and_animations();
    host.pump();
    host.load_images();
}

fn key(host: &mut Composed, key: slint::platform::Key) {
    let text: slint::SharedString = key.into();
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
    host.pump();
    host.load_images();
}

/// Every default theme screen in one direction, as `default[-rtl]-<screen>.png`.
fn shoot_default(host: &mut Composed, window: &MinimalSoftwareWindow, out: &Path, prefix: &str) {
    use slint::platform::Key;
    host.dispatch(Command::Navigate(Route::Home));
    host.load_images();
    shoot(window, &out.join(format!("{prefix}-home.png")));
    shoot_notices(host, window, out, prefix);
    // Down to the buttons under the game.
    key(host, Key::DownArrow);
    shoot(window, &out.join(format!("{prefix}-actions.png")));
    key(host, Key::UpArrow);
    // Up to the top bar, on to Library (left of Home from right to left), and in.
    key(host, Key::UpArrow);
    key(host, if prefix.ends_with("rtl") { Key::LeftArrow } else { Key::RightArrow });
    key(host, Key::Return);
    shoot(window, &out.join(format!("{prefix}-library.png")));
    // The list of sorts, then Back closes it.
    host.first("library-sort").invoke_accessible_default_action();
    host.pump();
    shoot(window, &out.join(format!("{prefix}-sort.png")));
    key(host, Key::Escape);
    key(host, Key::Return);
    // The Game Hub's first frame: its cover on the card it opens from.
    shoot_first(window, &out.join(format!("{prefix}-detail-opening.png")));
    shoot(window, &out.join(format!("{prefix}-detail.png")));
    key(host, Key::Escape);
    key(host, Key::Escape);
    for (route, name) in [
        (Route::Catalog, "catalog"),
        (Route::Downloads, "downloads"),
        (Route::Emulators, "emulators"),
        (Route::Themes, "themes"),
        (Route::Settings, "settings"),
    ] {
        host.dispatch(Command::Navigate(route));
        host.load_images();
        shoot(window, &out.join(format!("{prefix}-{name}.png")));
    }
    // A game the bundled catalog offers, when it offers any: its Game Hub with its releases.
    host.dispatch(Command::Navigate(Route::Catalog));
    if let Some(offer) = host.core().view().catalog.games.games.first().cloned() {
        host.dispatch(Command::OpenGame(offer.key.parse().unwrap()));
        host.load_images();
        shoot(window, &out.join(format!("{prefix}-offer.png")));
        // Down past its releases opens the technical details; Up closes them.
        let releases = host.core().view().detail.map_or(0, |d| d.releases.len());
        for _ in 0..=releases {
            key(host, Key::DownArrow);
        }
        shoot(window, &out.join(format!("{prefix}-offer-details.png")));
        key(host, Key::UpArrow);
        // Its first release: the sheet that asks first, then the download, read and waiting.
        // Once: in the other direction it is in Downloads already.
        let detail = host.core().view().detail.unwrap();
        if let Some(release) = detail.releases.first().filter(|r| r.download.is_empty()) {
            host.test.torrents.has(
                &release.id,
                "Sample",
                &[("Sample/game.rar", b"archive"), ("Sample/readme.txt", b"hi")],
            );
            host.test.archives.holds("game.rar", &[("game.pkg", b"package")]);
            let held = host.test.archives.hold();
            key(host, Key::DownArrow);
            key(host, Key::Return);
            shoot(window, &out.join(format!("{prefix}-download-sheet.png")));
            key(host, Key::Return);
            run_until(host, 500, |core| core.view().downloads.torrents.first().is_some_and(|t| t.state == "ready"));
            follow(host);
            shoot(window, &out.join(format!("{prefix}-downloads-torrent.png")));
            // The keys on the download's first button.
            key(host, Key::DownArrow);
            shoot(window, &out.join(format!("{prefix}-downloads-keys.png")));
            // Downloaded: its archive unpacks. Once: the next theme finds it unpacked.
            let first = host.core().view().downloads.torrents.first().is_some_and(|t| t.state == "ready");
            if first {
                key(host, Key::Return);
                // Bytes arrive: the card says the time left, and Open folder follows its buttons.
                host.test.torrents.progress(&release.id, 4);
                host.test.torrents.rate(&release.id, 1);
                run_until(host, 1000, |core| {
                    core.view().downloads.torrents.first().is_some_and(|t| t.seconds_left > 0)
                });
                host.ui.render().unwrap();
                shoot(window, &out.join(format!("{prefix}-downloads-downloading.png")));
                host.test.torrents.complete(&release.id);
                run_until(host, 1000, |core| {
                    core.view().downloads.torrents.first().is_some_and(|t| t.unpack == "unpacking")
                });
                host.pump();
                host.ui.render().unwrap();
                // The card's buttons follow its actions on a drawn window, not only on the
                // testing backend: Pause, drawn while it downloaded, is gone.
                let shown = ["pause", "stop-seeding", "cancel"].map(|a| host.count(&format!("torrent-action:{a}")));
                assert_eq!(shown, [0, 1, 1]);
                shoot(window, &out.join(format!("{prefix}-downloads-unpacking.png")));
                held.release();
                run_until(host, 1000, |core| {
                    core.view().downloads.torrents.first().is_some_and(|t| t.unpack == "unpacked")
                });
                host.ui.render().unwrap();
                shoot(window, &out.join(format!("{prefix}-downloads-unpacked.png")));
            }
            key(host, Key::Escape);
        }
        key(host, Key::Escape);
    }
    shoot_vpn(host, window, out, prefix);
    shoot_install_sheet(host, window, out, prefix);
    shoot_resolving(host, window, out, prefix);
    // KytyPS5's page, then down to its settings: Resolution, Video out and Present mode.
    host.dispatch(Command::Navigate(Route::Plugin("emu-kyty".parse().unwrap())));
    host.load_images();
    shoot(window, &out.join(format!("{prefix}-plugin.png")));
    for _ in 0..9 {
        key(host, Key::DownArrow);
    }
    shoot(window, &out.join(format!("{prefix}-plugin-settings.png")));
    // The Game Catalog's page: its settings, the catalog sources and the catalog refresh.
    host.dispatch(Command::Navigate(Route::Plugin("release-catalog".parse().unwrap())));
    host.load_images();
    shoot(window, &out.join(format!("{prefix}-catalog-plugin.png")));
    // A theme's sheet beside Themes: use it, and its options.
    host.dispatch(Command::Navigate(Route::Themes));
    host.dispatch(Command::Navigate(Route::Plugin("default".parse().unwrap())));
    host.load_images();
    shoot(window, &out.join(format!("{prefix}-theme.png")));
    host.dispatch(Command::Navigate(Route::Home));
}

/// The install sheet of the finished download, as `<prefix>-install-sheet.png`, then with the
/// core's refusal of a folder outside the game folders, as `<prefix>-install-sheet-error.png`.
fn shoot_install_sheet(host: &mut Composed, window: &MinimalSoftwareWindow, out: &Path, prefix: &str) {
    host.dispatch(Command::Navigate(Route::Downloads));
    host.load_images();
    // The screen's change handler runs on the next pass, and resets the screen's keys.
    slint::platform::update_timers_and_animations();
    let Some(t) = host.core().view().downloads.torrents.into_iter().find(|t| t.actions.iter().any(|a| a == "install"))
    else {
        return;
    };
    let install =
        i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), "torrent-action:install")
            .find(|e| e.accessible_value().as_deref() == Some(t.id.as_str()))
            .expect("the download's Install");
    install.invoke_accessible_default_action();
    host.pump();
    shoot(window, &out.join(format!("{prefix}-install-sheet.png")));
    let outside =
        Command::Torrent(nexus_core::TorrentCmd::Install { id: t.id.clone(), folder: Some("/elsewhere".into()) });
    assert!(host.rt.block_on(host.test.core.dispatch(outside)).is_err(), "a folder outside the game folders");
    host.ui.render().unwrap();
    shoot(window, &out.join(format!("{prefix}-install-sheet-error.png")));
    key(host, slint::platform::Key::Escape);
}

/// The second offer's download while it looks up its files, as `<prefix>-downloads-resolving.png`:
/// its first button says so and does nothing. It leaves no download behind.
fn shoot_resolving(host: &mut Composed, window: &MinimalSoftwareWindow, out: &Path, prefix: &str) {
    host.dispatch(Command::Navigate(Route::Catalog));
    let games = host.core().view().catalog.games.games;
    let Some(offer) = games.iter().skip(1).find(|g| g.platform != "PC").cloned() else { return };
    host.dispatch(Command::OpenGame(offer.key.parse().unwrap()));
    let Some(release) = host.core().view().detail.and_then(|d| d.releases.first().cloned()) else { return };
    host.test.torrents.has(&release.id, "Sample", &[("Sample/game.pkg", b"package")]);
    let held = host.test.torrents.hold();
    host.dispatch(Command::Torrent(nexus_core::TorrentCmd::Download {
        offer: offer.key.parse().unwrap(),
        release: release.id.clone(),
        folder: None,
    }));
    host.dispatch(Command::Navigate(Route::Downloads));
    host.load_images();
    shoot(window, &out.join(format!("{prefix}-downloads-resolving.png")));
    held.release();
    run_until(host, 1000, |core| {
        core.view().downloads.torrents.iter().any(|t| t.id == release.id && t.state == "ready")
    });
    host.dispatch(Command::Torrent(nexus_core::TorrentCmd::Act {
        id: release.id,
        action: nexus_core::TorrentAction::Remove,
    }));
}

/// Three notices on Home, one of each kind, as `<prefix>-notices.png`. The core makes a
/// success only for a real download and an info only for a real launch, so the shot writes the
/// notices straight into the theme's `Vm`, then closes them.
fn shoot_notices(host: &mut Composed, window: &MinimalSoftwareWindow, out: &Path, prefix: &str) {
    use slint_interpreter::{Struct, Value};
    let card = |id: f64, text: &str, kind: &str, opens: &str| {
        let fields: [(&str, Value); 5] = [
            ("id", Value::Number(id)),
            ("text", Value::String(text.into())),
            ("kind", Value::String(kind.into())),
            ("art", Value::Image(slint::Image::default())),
            ("opens", Value::String(opens.into())),
        ];
        Value::Struct(fields.into_iter().map(|(k, v)| (k.to_owned(), v)).collect::<Struct>())
    };
    let cards = vec![
        card(9001.0, "The trailer opened in your browser", "info", ""),
        card(9002.0, "Download complete: Astro Drift. Install it from Downloads.", "success", "Open Downloads"),
        card(9003.0, "Can't start Lumen Gate: firmware missing", "error", "Open the Game Hub"),
    ];
    let model = slint::ModelRc::new(slint::VecModel::from(cards));
    host.instance().set_global_property("Vm", "notice-cards", Value::Model(model)).unwrap();
    shoot(window, &out.join(format!("{prefix}-notices.png")));
    host.instance().set_global_property("Vm", "notice-cards", Value::Model(slint::ModelRc::default())).unwrap();
}

/// "Download only through a VPN": Settings at its DOWNLOADS rows with two VPNs, then the
/// second offer's download sheet with the VPN connected and without it, and its download
/// waiting for the VPN, as `<prefix>-settings-downloads[-end].png`, `<prefix>-download-sheet-vpn.png`,
/// `<prefix>-download-sheet-no-vpn.png` and `<prefix>-downloads-waiting.png`. It leaves no
/// download behind, so the other direction shoots the same.
fn shoot_vpn(host: &mut Composed, window: &MinimalSoftwareWindow, out: &Path, prefix: &str) {
    use slint::platform::Key;
    host.test.network.up("wg-se");
    host.test.network.down("work");
    host.dispatch(Command::Vpn(nexus_core::VpnCmd::SetOnly(true)));
    host.dispatch(Command::Navigate(Route::Settings));
    host.load_images();
    // Down past GAMES, UPDATES' two rows and LAUNCHER's six to the VPN switch.
    for _ in 0..host.core().view().settings.folders.len() + 9 {
        key(host, Key::DownArrow);
    }
    shoot(window, &out.join(format!("{prefix}-settings-downloads.png")));
    // Down past the VPNs and "Add a VPN" to sharing.
    for _ in 0..4 {
        key(host, Key::DownArrow);
    }
    shoot(window, &out.join(format!("{prefix}-settings-downloads-end.png")));
    key(host, Key::Escape);

    host.dispatch(Command::Navigate(Route::Catalog));
    let games = host.core().view().catalog.games.games;
    let Some(offer) = games.iter().skip(1).find(|g| g.platform != "PC").cloned() else { return };
    host.dispatch(Command::OpenGame(offer.key.parse().unwrap()));
    let Some(release) = host.core().view().detail.and_then(|d| d.releases.first().cloned()) else { return };
    host.test.torrents.has(&release.id, "Sample", &[("Sample/game.pkg", b"package")]);
    host.load_images();
    // Down to its first release, as on the first offer, and Return opens the sheet.
    key(host, Key::DownArrow);
    key(host, Key::Return);
    shoot(window, &out.join(format!("{prefix}-download-sheet-vpn.png")));
    // A typed folder outside the packages folders: the core refuses, and the sheet says why.
    key(host, Key::UpArrow);
    key(host, Key::Return);
    for c in "/elsewhere".chars() {
        let text: slint::SharedString = c.to_string().into();
        host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
        host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
    }
    key(host, Key::Return);
    let text: slint::SharedString = Key::Return.into();
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
    let refused = host.requests.borrow_mut().drain(..).collect::<Vec<_>>();
    for request in refused {
        if let nexus_ui_host::UiRequest::Core(command) = request {
            assert!(host.rt.block_on(host.test.core.dispatch(command)).is_err(), "the core refuses the folder");
        }
    }
    host.ui.render().unwrap();
    shoot(window, &out.join(format!("{prefix}-download-sheet-error.png")));
    // The VPN drops: the setting reads it again. The keys are still on the release.
    key(host, Key::Escape);
    host.test.network.down("wg-se");
    host.dispatch(Command::Vpn(nexus_core::VpnCmd::SetOnly(true)));
    key(host, Key::Return);
    shoot(window, &out.join(format!("{prefix}-download-sheet-no-vpn.png")));
    key(host, Key::Return);
    follow(host);
    shoot(window, &out.join(format!("{prefix}-downloads-waiting.png")));

    // Off: the download reads its files, then goes.
    host.dispatch(Command::Vpn(nexus_core::VpnCmd::SetOnly(false)));
    run_until(host, 1000, |core| {
        core.view().downloads.torrents.iter().any(|t| t.id == release.id && t.state == "ready")
    });
    if host.core().view().downloads.torrents.iter().any(|t| t.id == release.id) {
        host.dispatch(Command::Torrent(nexus_core::TorrentCmd::Act {
            id: release.id,
            action: nexus_core::TorrentAction::Remove,
        }));
    }
    key(host, Key::Escape);
    key(host, Key::Escape);
}

/// Settings at its LAUNCHER rows, with "Size of everything" focused and then "Start at login",
/// then Home at the smallest and the largest size, as `default-settings-launcher.png`,
/// `default-settings-login.png` and `default-home-<percent>.png`.
fn shoot_sizes(host: &mut Composed, window: &MinimalSoftwareWindow, out: &Path) {
    use slint::platform::Key;
    host.dispatch(Command::Navigate(Route::Settings));
    host.load_images();
    // Down past the game folders, Rescan, UPDATES' two rows and Full screen.
    for _ in 0..host.core().view().settings.folders.len() + 4 {
        key(host, Key::DownArrow);
    }
    shoot(window, &out.join("default-settings-launcher.png"));
    // Down past Back to the launcher and Interface sounds to Start at login.
    for _ in 0..3 {
        key(host, Key::DownArrow);
    }
    shoot(window, &out.join("default-settings-login.png"));
    key(host, Key::Escape);
    for (size, name) in [(0.8, "80"), (1.4, "140")] {
        host.dispatch(Command::SetScale(size));
        host.dispatch(Command::Navigate(Route::Home));
        host.load_images();
        shoot(window, &out.join(format!("default-home-{name}.png")));
    }
    host.dispatch(Command::SetScale(1.0));
}

/// A picture `width` by `height` that fades from `from` to `to`, as PNG bytes.
fn picture(width: u32, height: u32, from: [u8; 3], to: [u8; 3]) -> Vec<u8> {
    let image = image::RgbImage::from_fn(width, height, |x, y| {
        let t = (x + y) as f32 / (width + height) as f32;
        image::Rgb(std::array::from_fn(|i| (from[i] as f32 + (to[i] as f32 - from[i] as f32) * t) as u8))
    });
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

/// Pictures at the sample catalogs' made-up Steam art addresses.
fn serve_steam_art(host: &Composed) {
    for (app, from, to) in [(3000001, [40, 90, 160], [10, 20, 50]), (3000002, [170, 90, 40], [40, 15, 10])] {
        let art = format!("https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/{app}");
        host.test.http.serve(&format!("{art}/library_600x900_2x.jpg"), picture(600, 900, from, to));
        host.test.http.serve(&format!("{art}/library_hero.jpg"), picture(1920, 620, to, from));
        host.test.http.serve(&format!("{art}/header.jpg"), picture(920, 430, from, to));
        for shot in ["ss_1", "ss_2"] {
            host.test.http.serve(&format!("{art}/{shot}.jpg"), picture(1280, 720, to, from));
        }
    }
}

/// The offers with Steam data: a PS5 package's Game Hub and its sheet, and a PC game packed
/// with Wine with its ProtonDB card and its menu.
fn shoot_steam_offers(host: &mut Composed, window: &MinimalSoftwareWindow, out: &Path) {
    use slint::platform::Key;
    host.dispatch(Command::Navigate(Route::Catalog));
    let offers = host.core().view().catalog.games.games;
    let key_of = |title: &str| offers.iter().find(|o| o.title == title).map(|o| o.key.clone()).unwrap();
    host.dispatch(Command::OpenGame(key_of("Sample Without Title Id").parse().unwrap()));
    host.load_images();
    shoot(window, &out.join("default-offer-package.png"));
    key(host, Key::DownArrow);
    key(host, Key::Return);
    shoot(window, &out.join("default-download-sheet-package.png"));
    key(host, Key::Escape);
    key(host, Key::Escape);
    host.dispatch(Command::OpenGame(key_of("Sample Windows Game").parse().unwrap()));
    host.load_images();
    shoot(window, &out.join("default-offer-pc.png"));
    key(host, Key::Menu);
    shoot(window, &out.join("default-offer-pc-menu.png"));
    key(host, Key::Escape);
    key(host, Key::Escape);
    host.dispatch(Command::Navigate(Route::Home));
}

/// A PC with a strong graphics card, or one whose driver is too old for Vulkan 1.3.
fn this_pc(vulkan: (u32, u32)) -> nexus_core::Specs {
    let gpu = nexus_core::Gpu {
        name: "Test GPU".into(),
        memory_gb: Some(24.0),
        discrete: true,
        unified: false,
        vulkan: Some(vulkan),
    };
    nexus_core::Specs {
        cpu: "Test CPU".into(),
        cores: 16,
        threads: 32,
        memory_gb: 64.0,
        avx2: Some(true),
        gpus: Some(vec![gpu]),
    }
}

/// A PS5 offer on a PC that can't start it, with the notice at the start and the sheet's
/// warnings, then on a PC where it doesn't fit, as `default-this-pc-*.png`.
fn shoot_this_pc(window: &MinimalSoftwareWindow, out: &Path) {
    use slint::platform::Key;
    for (name, vulkan) in [("no", (1, 2)), ("space", (1, 3))] {
        let home = tempfile::tempdir().unwrap();
        let mut host = composed_on_current_platform(|h| {
            h.home(home.path())
                .hardware(this_pc(vulkan))
                .free_space(10_000_000_000)
                .plugin(offering_emulator("vulkan = \"1.3\""))
                .built_in(nexus_api::PluginByKind::service(release_catalog::ReleaseCatalog::default()))
        });
        host.ui.start().unwrap();
        let catalog = serde_json::json!({ "complete": true, "translation": { "language": "en" }, "topic_count": 1,
            "topics": [{ "id": "1", "title": "[PS5] Astro Drift [EUR / v01.05]", "size": "50 GB",
                "magnet": "magnet:?xt=urn:btih:c12fe1c06bba254a9dc9f519b335aa7c1367a88a",
                "game_info": { "name": "Astro Drift", "title_id": "PPSA01234", "version": "v01.05", "region": "EUR" } }] });
        host.test.http.serve("https://xyz.dev/c/ps5.json", catalog.to_string());
        host.dispatch(Command::SetConfig {
            plugin: id("release-catalog"),
            scope: nexus_api::ConfigScope::Emulator,
            value: serde_json::json!({ "catalog_url": "https://xyz.dev/c" }),
        });
        host.catalogs_fetched();
        host.dispatch(Command::RefreshLibrary);
        host.dispatch(Command::Navigate(Route::Home));
        host.load_images();
        shoot(window, &out.join(format!("default-this-pc-{name}-start.png")));
        host.dispatch(Command::OpenGame("release-catalog:emu-kyty/PPSA01234".parse().unwrap()));
        host.load_images();
        shoot(window, &out.join(format!("default-this-pc-{name}.png")));
        key(&mut host, Key::DownArrow);
        key(&mut host, Key::Return);
        shoot(window, &out.join(format!("default-this-pc-{name}-sheet.png")));
        assert_served(&host, &[]);
    }
}

/// A Game Hub's releases as `default[-rtl]-releases.png`: long titles, New tags, a warning, and
/// the keys on the second row. Each row's Download button ends at the same place.
fn shoot_releases(window: &MinimalSoftwareWindow, out: &Path) {
    use slint::platform::Key;
    for (name, direction) in [("default", "ltr"), ("default-rtl", "rtl")] {
        let home = tempfile::tempdir().unwrap();
        let mut host = composed_on_current_platform(|h| {
            h.home(home.path())
                .plugin(offering_emulator("vulkan = \"1.3\""))
                .built_in(nexus_api::PluginByKind::service(release_catalog::ReleaseCatalog::default()))
        });
        host.ui.start().unwrap();
        host.dispatch(Command::SetThemeOption { key: "direction".into(), value: direction.into() });
        // The first two from this week: New.
        let topic = |id: u32, flag: &str, version: &str, region: &str| {
            let added = crate::releases::days_ago(if id <= 2 { 4 } else { 24 });
            serde_json::json!({ "id": id.to_string(), "title": format!("[PS5] Astro Drift [{region} / {version} / {flag}]"),
                "published_at": added,
                "size": format!("{id}.4 GB"), "magnet": format!("magnet:?xt=urn:btih:{id:040x}"),
                "game_info": { "name": "Astro Drift", "title_id": "PPSA01234", "version": version, "region": region } })
        };
        let catalog = serde_json::json!({ "complete": true, "translation": { "language": "en" }, "topic_count": 4,
            "topics": [
                topic(1, "fpkg", "1.000.010 (dated 30.03.2026)", "EU² / US² / JP² / AS² / INT(GLO)² / EU¹ / US¹ / JP¹ / AS¹"),
                topic(2, "folder", "1.000.010 (dated 30.03.2026)", "EU¹ / US¹ / JP¹ / AS¹ / INT(GLO)¹"),
                topic(3, "fpkg", "v01.000.000 (v01.000.010)", "EUR"),
                topic(4, "folder", "v01.000.000 (v01.000.010)", "EUR"),
            ] });
        host.test.http.serve("https://xyz.dev/c/ps5.json", catalog.to_string());
        host.dispatch(Command::SetConfig {
            plugin: id("release-catalog"),
            scope: nexus_api::ConfigScope::Emulator,
            value: serde_json::json!({ "catalog_url": "https://xyz.dev/c" }),
        });
        host.catalogs_fetched();
        host.dispatch(Command::RefreshLibrary);
        host.dispatch(Command::OpenGame("release-catalog:emu-kyty/PPSA01234".parse().unwrap()));
        host.load_images();
        key(&mut host, Key::DownArrow);
        key(&mut host, Key::DownArrow);
        shoot(window, &out.join(format!("{name}-releases.png")));
    }
}

/// The Library's header with a controller, as `default-pad-hints.png`, and its key hints cut
/// out at four times the size as `default-pad-hints-zoom.png`: the d-pad's two pairs, ⮜  ⮞ and
/// ⮝  ⮟, are the same size.
fn shoot_pad_hints(window: &MinimalSoftwareWindow, out: &Path) {
    use nexus_ui_host::PadButton;
    let mut host = composed_on_current_platform(|h| h.plugin(games("emu-a", vec!["Alpha", "Beta"])));
    host.ui.start().unwrap();
    host.dispatch(Command::RefreshLibrary);
    host.load_images();
    let pad = |host: &mut Composed, button: PadButton| {
        host.ui.pad(button).unwrap();
        follow(host);
        follow(host);
    };
    // Up to the Library tab, in, then up to its header.
    for button in [PadButton::Up, PadButton::Right, PadButton::South, PadButton::Up] {
        pad(&mut host, button);
    }
    let full = out.join("default-pad-hints.png");
    shoot(window, &full);
    let image = image::open(&full).unwrap();
    let (w, h) = (image.width(), image.height());
    let hints = image.crop_imm(w / 2, h - 90, w / 2, 90);
    hints
        .resize(hints.width() * 4, hints.height() * 4, image::imageops::FilterType::Nearest)
        .save(out.join("default-pad-hints-zoom.png"))
        .unwrap();
}

/// The Library's genres in a narrow window, with the keys on the last chip, left to right and
/// right to left, as `default[-rtl]-genres.png`: the row scrolls, and fades where chips hide.
fn shoot_genres(window: &MinimalSoftwareWindow, out: &Path) {
    use slint::platform::Key;
    window.set_size(slint::PhysicalSize::new(640, 900));
    for (direction, prefix) in [("ltr", "default"), ("rtl", "default-rtl")] {
        let mut host = composed_on_current_platform(|h| {
            h.plugin(games("emu-a", vec!["Alpha"])).plugin(store(
                "store",
                vec![("CUSA00000", 4.5, "")],
                None,
                Duration::ZERO,
            ))
        });
        host.ui.start().unwrap();
        host.dispatch(Command::SetScale(1.4));
        host.dispatch(Command::SetThemeOption { key: "direction".into(), value: direction.into() });
        host.dispatch(Command::RefreshLibrary);
        host.until(Area::Library, |core| core.view().home.library.genres.len() >= 3);
        host.load_images();
        // Up to the Library tab, in, then up to the genres and on to the last one.
        key(&mut host, Key::UpArrow);
        key(&mut host, if direction == "rtl" { Key::LeftArrow } else { Key::RightArrow });
        key(&mut host, Key::Return);
        shoot(window, &out.join(format!("{prefix}-genres-start.png")));
        key(&mut host, Key::UpArrow);
        for _ in 0..2 {
            key(&mut host, if direction == "rtl" { Key::LeftArrow } else { Key::RightArrow });
        }
        shoot(window, &out.join(format!("{prefix}-genres.png")));
    }
    window.set_size(slint::PhysicalSize::new(WIDTH, HEIGHT));
}

/// A trailer in the window: playing with its controls, playing once they hid, and paused, left
/// to right and right to left, as `default[-rtl]-trailer[-hidden|-paused].png`. No frame comes,
/// so the picture is black.
fn shoot_trailer(window: &MinimalSoftwareWindow, out: &Path) {
    use nexus_ui_host::TrailerStatus;
    for (direction, prefix) in [("ltr", "default"), ("rtl", "default-rtl")] {
        let mut host = composed_on_current_platform(|h| {
            h.plugin(games("emu-a", vec!["Alpha"])).plugin(store(
                "store",
                vec![("CUSA00000", 4.5, "")],
                None,
                Duration::ZERO,
            ))
        });
        host.ui.set_trailers_available(true);
        host.ui.start().unwrap();
        host.dispatch(Command::SetThemeOption { key: "direction".into(), value: direction.into() });
        host.dispatch(Command::RefreshLibrary);
        host.until(Area::Library, |core| core.view().detail.is_some_and(|d| !d.trailer_url.is_empty()));
        host.load_images();
        let text: slint::SharedString = "t".into();
        host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text });
        let playing = TrailerStatus { playing: true, loading: false, position: 42.0, duration: 130.0 };
        host.ui.set_trailer(Some(playing.clone()));
        host.ui.render().unwrap();
        shoot(window, &out.join(format!("{prefix}-trailer.png")));
        // The controls hide 3 s after the last key.
        for _ in 0..7 {
            pass(Duration::from_millis(500));
            slint::platform::update_timers_and_animations();
        }
        shoot(window, &out.join(format!("{prefix}-trailer-hidden.png")));
        host.ui.set_trailer(Some(TrailerStatus { playing: false, ..playing }));
        host.ui.render().unwrap();
        shoot(window, &out.join(format!("{prefix}-trailer-paused.png")));
    }
}

/// The on-screen keyboard over the Library's search, opened with the controller: letters with
/// Shift on as `default-keyboard.png`, the symbols as `default-keyboard-symbols.png`, then at
/// the smallest and largest size, at a 1.5 scale factor and right to left, as
/// `default-keyboard-<percent>.png`, `default-keyboard-scale-150.png` and
/// `default-rtl-keyboard.png`. The keys are drawn on whole pixels at every one of them.
fn shoot_keyboard(window: &MinimalSoftwareWindow, out: &Path) {
    use nexus_ui_host::PadButton;
    let mut host = composed_on_current_platform(|h| h.plugin(games_on("emu-a", "PS5", vec!["Astro Drift"])));
    host.ui.start().unwrap();
    host.dispatch(Command::RefreshLibrary);
    let pad = |host: &mut Composed, button: PadButton| {
        host.ui.pad(button).unwrap();
        follow(host);
        follow(host);
    };
    for button in [PadButton::Up, PadButton::Right, PadButton::South, PadButton::North, PadButton::South] {
        pad(&mut host, button);
    }
    pad(&mut host, PadButton::LeftShoulder);
    shoot(window, &out.join("default-keyboard.png"));
    pad(&mut host, PadButton::RightShoulder);
    shoot(window, &out.join("default-keyboard-symbols.png"));
    pad(&mut host, PadButton::RightShoulder);
    for (size, name) in [(0.8, "80"), (1.4, "140")] {
        host.dispatch(Command::SetScale(size));
        shoot(window, &out.join(format!("default-keyboard-{name}.png")));
    }
    host.dispatch(Command::SetScale(1.0));
    // The same window on a screen with 1.5 pixels to the point.
    window.dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor: 1.5 });
    window.set_size(slint::PhysicalSize::new(WIDTH * 3 / 2, HEIGHT * 3 / 2));
    host.ui.render().unwrap();
    shoot(window, &out.join("default-keyboard-scale-150.png"));
    window.dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor: 1.0 });
    window.set_size(slint::PhysicalSize::new(WIDTH, HEIGHT));
    host.dispatch(Command::SetThemeOption { key: "direction".into(), value: "rtl".into() });
    shoot(window, &out.join("default-rtl-keyboard.png"));
}

/// The machine's settings as the whole screen, over a fake system plugin: the top bar's icons
/// with the network panel open as `default-system-network.png`, the sound and Bluetooth panels,
/// the panel at the smallest and largest size and at a 1.5 scale factor, and the USB stick's
/// confirmation as `default-system-install.png`.
fn shoot_system(window: &MinimalSoftwareWindow, out: &Path) {
    use nexus_core::system::{Place, SystemCommand};
    let system = nexus_testkit::FakeSystem::new();
    system.with(|m| {
        m.network =
            nexus_api::NetworkStatus { connection: nexus_api::Connection::Wifi, name: "Home".into(), signal: Some(60) };
        m.wifi = vec![
            nexus_api::WifiNetwork { ssid: "Home".into(), signal: 60, secured: true, saved: true, active: true },
            nexus_api::WifiNetwork { ssid: "Cafe".into(), signal: 25, secured: false, saved: false, active: false },
            nexus_api::WifiNetwork { ssid: "Neighbour".into(), signal: 90, secured: true, saved: false, active: false },
        ];
        m.bluetooth_on = true;
        m.devices = vec![
            nexus_api::BluetoothDevice {
                address: "AA".into(),
                name: "DualSense".into(),
                paired: true,
                connected: true,
            },
            nexus_api::BluetoothDevice { address: "BB".into(), name: "Headset".into(), paired: true, connected: false },
        ];
        m.outputs = vec![
            nexus_api::AudioOutput { name: "hdmi".into(), description: "TV or monitor (HDMI)".into(), current: true },
            nexus_api::AudioOutput { name: "hp".into(), description: "Headphones".into(), current: false },
        ];
        m.level = nexus_api::AudioLevel { volume: 65, muted: false };
        m.battery = Some(nexus_api::Battery { percent: 18, charging: false });
        m.disks = vec![nexus_api::InstallDisk {
            path: "/dev/nvme0n1".into(),
            bytes: 512_000_000_000,
            windows: true,
            usb: false,
            model: "Samsung 970".into(),
        }];
    });
    let place = Place { whole_screen: true, nexus_os: true, stick: false };
    let plugin = system.plugin();
    let mut host = composed_on_current_platform(|h| {
        h.place(place).plugin(plugin).plugin(games_on("emu-a", "PS5", vec!["Astro Drift"]))
    });
    host.ui.start().unwrap();
    host.dispatch(Command::RefreshLibrary);
    host.dispatch(Command::System(SystemCommand::Refresh));
    let open = |host: &mut Composed, label: &str| {
        let found = i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), label);
        found.into_iter().next().expect("the icon").invoke_accessible_default_action();
        follow(host);
        follow(host);
    };
    open(&mut host, "status:network");
    shoot(window, &out.join("default-system-network.png"));
    open(&mut host, "status:sound");
    shoot(window, &out.join("default-system-sound.png"));
    open(&mut host, "status:bluetooth");
    shoot(window, &out.join("default-system-bluetooth.png"));
    open(&mut host, "status:network");
    for (size, name) in [(0.8, "80"), (1.4, "140")] {
        host.dispatch(Command::SetScale(size));
        shoot(window, &out.join(format!("default-system-{name}.png")));
    }
    host.dispatch(Command::SetScale(1.0));
    window.dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor: 1.5 });
    window.set_size(slint::PhysicalSize::new(WIDTH * 3 / 2, HEIGHT * 3 / 2));
    host.ui.render().unwrap();
    shoot(window, &out.join("default-system-scale-150.png"));
    window.dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor: 1.0 });
    window.set_size(slint::PhysicalSize::new(WIDTH, HEIGHT));
    drop(host);

    let place = Place { whole_screen: true, nexus_os: true, stick: true };
    let plugin = system.plugin();
    let mut host = composed_on_current_platform(|h| h.place(place).plugin(plugin));
    host.ui.start().unwrap();
    host.dispatch(Command::RefreshLibrary);
    host.dispatch(Command::System(SystemCommand::Refresh));
    follow(&mut host);
    for key in [slint::platform::Key::DownArrow, slint::platform::Key::Return, slint::platform::Key::Return] {
        let text: slint::SharedString = key.into();
        host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
        host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
        follow(&mut host);
    }
    shoot(window, &out.join("default-system-install.png"));
}

/// The launch splash of an emulator that lists its keyboard keys, left to right and right to
/// left, as `default[-rtl]-splash-keys.png`.
fn shoot_splash(window: &MinimalSoftwareWindow, out: &Path) {
    let keymap: String = emu_kyty::KEYS
        .iter()
        .map(|(button, keys)| format!("[[keymap]]\nbutton = \"{button}\"\nkeys = \"{keys}\"\n"))
        .collect();
    for (direction, prefix) in [("ltr", "default"), ("rtl", "default-rtl")] {
        let keymap = keymap.clone();
        let mut host =
            composed_on_current_platform(move |h| h.plugin(playable_games_with("emu-a", vec!["Alpha"], &keymap)));
        host.ui.start().unwrap();
        host.dispatch(Command::SetThemeOption { key: "direction".into(), value: direction.into() });
        host.dispatch(Command::RefreshLibrary);
        // No window yet: the game stays "starting", behind the splash.
        host.test.desktop.no_windows();
        host.dispatch(Command::Launch("emu-a:G0".parse().unwrap()));
        host.load_images();
        shoot(window, &out.join(format!("{prefix}-splash-keys.png")));
    }
}

/// Emulators named by their console, left to right and right to left: the first start's steps
/// as `default[-rtl]-names-welcome.png`, the Emulators list with each build as
/// `default[-rtl]-names-emulators.png`, and a first play's prompt as
/// `default[-rtl]-names-install-prompt.png`.
fn shoot_console_names(window: &MinimalSoftwareWindow, out: &Path) {
    for (direction, prefix) in [("ltr", "default"), ("rtl", "default-rtl")] {
        let mut host = composed_on_current_platform(|h| {
            h.first_run()
                .plugin(console_emulator("emu-k", "PS5", "KytyPS5", "KytyPS5-2026-10-21-e2dd48a", vec!["Astro Drift"]))
                .plugin(console_emulator("emu-s", "PS4", "shadPS4", "v.0.20.0", vec![]))
        });
        host.ui.start().unwrap();
        host.dispatch(Command::SetThemeOption { key: "direction".into(), value: direction.into() });
        host.dispatch(Command::RefreshLibrary);
        host.load_images();
        shoot(window, &out.join(format!("{prefix}-names-welcome.png")));
        host.dispatch(Command::FinishWelcome);
        host.dispatch(Command::Navigate(Route::Emulators));
        host.load_images();
        shoot(window, &out.join(format!("{prefix}-names-emulators.png")));
        drop(host);
        // KytyPS5 not installed yet: a first play asks to install it.
        let mut host = composed_on_current_platform(|h| {
            h.plugin(console_emulator("emu-k", "PS5", "KytyPS5", "", vec!["Astro Drift"]))
        });
        host.ui.start().unwrap();
        host.dispatch(Command::SetThemeOption { key: "direction".into(), value: direction.into() });
        host.dispatch(Command::RefreshLibrary);
        host.dispatch(Command::Launch("emu-k:G0".parse().unwrap()));
        host.load_images();
        shoot(window, &out.join(format!("{prefix}-names-install-prompt.png")));
    }
}

/// PS4 box art: a band of PlayStation blue with a white "PS4" across the top, then artwork.
fn ps4_box_art() -> Vec<u8> {
    let image = image::RgbImage::from_fn(600, 900, |x, y| match y {
        0..80 if (40..160).contains(&x) && (25..55).contains(&y) => image::Rgb([255, 255, 255]),
        0..80 => image::Rgb([10, 80, 190]),
        _ => image::Rgb([150 + (x / 8) as u8, 60, 40 + (y / 10) as u8]),
    });
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

/// A PS4 game whose store art is box art with the blue band, and whose community result says
/// when it was tested: its card and its Game Hub, as `default-ps4-*.png`.
fn shoot_ps4_cover(window: &MinimalSoftwareWindow, out: &Path) {
    use slint::platform::Key;
    let mut host = composed_on_current_platform(|h| {
        h.plugin(tested_games("emu-b", vec!["Paper Knights", "Quiet Orbit"])).plugin(store(
            "store",
            vec![("CUSA00000", 4.5, "")],
            Some("https://art.example/ps4.png".into()),
            Duration::ZERO,
        ))
    });
    host.test.http.serve("https://art.example/ps4.png", ps4_box_art());
    host.ui.start().unwrap();
    host.dispatch(Command::RefreshLibrary);
    host.dispatch(Command::Navigate(Route::Home));
    host.load_images();
    shoot(window, &out.join("default-ps4-home.png"));
    key(&mut host, Key::Return);
    host.load_images();
    shoot(window, &out.join("default-ps4-detail.png"));
    // The same Game Hub from right to left: its buttons, the trailer's among them, and its facts.
    host.dispatch(Command::SetThemeOption { key: "direction".into(), value: "rtl".into() });
    host.load_images();
    shoot(window, &out.join("default-rtl-ps4-detail.png"));
    host.dispatch(Command::SetThemeOption { key: "direction".into(), value: "ltr".into() });
    // The stub's screenshots: this picture is about the cover, so the Game Hub shows none.
    assert_served(&host, &["https://art.example/1.png".into(), "https://art.example/2.png".into()]);
}

/// Fails when `host` asked for a URL its fake web server doesn't serve, other than `missing`.
/// The fake answers those with a 404, and a plugin shows that as a notice on the screenshots.
fn assert_served(host: &Composed, missing: &[String]) {
    let unserved = host.test.http.unserved();
    assert_eq!(unserved, missing, "the screenshot host asked for URLs it doesn't serve");
}

/// This test thread's offscreen window (Slint's platform is per thread, so each test has its
/// own), and the folder the pictures go to.
fn offscreen() -> (Rc<MinimalSoftwareWindow>, std::path::PathBuf) {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Offscreen(window.clone()))).unwrap();
    window.set_size(slint::PhysicalSize::new(WIDTH, HEIGHT));
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("screenshots");
    std::fs::create_dir_all(&out).unwrap();
    (window, out)
}

// Each part that builds its own library is its own test, so they render at the same time.
macro_rules! shots {
    ($($test:ident => $shoot:ident),* $(,)?) => {$(
        #[test]
        fn $test() {
            let (window, out) = offscreen();
            $shoot(&window, &out);
        }
    )*};
}

shots! {
    this_pc_screens => shoot_this_pc,
    the_releases => shoot_releases,
    a_ps4_cover => shoot_ps4_cover,
    genres => shoot_genres,
    the_trailer => shoot_trailer,
    the_splash => shoot_splash,
    console_names => shoot_console_names,
    the_keyboard => shoot_keyboard,
    the_pad_hints => shoot_pad_hints,
    the_system_controls => shoot_system,
}

/// The library the default screens show: a home folder (for the packages folder downloads go
/// to), emulators on each console, the real KytyPS5 plugin and game catalog with the sample
/// catalogs, a game selected and a search typed.
fn library(home: &Path) -> Composed {
    let mut host = composed_on_current_platform(|h| {
        h.home(home)
            .hardware(this_pc((1, 3)))
            .discover(&fixture_themes())
            .plugin(games_on("emu-a", "PS5", vec!["Astro Drift", "Deep Lantern", "Ember Peaks", "Neon Harbor"]))
            .plugin(games_on("emu-b", "PS4", vec!["Paper Knights", "Quiet Orbit"]))
            // The real KytyPS5 plugin, for its Settings section and Emulators block.
            .built_in(nexus_api::PluginByKind::emulator(emu_kyty::KytyPlugin))
            .plugin(games_needing_an_emulator("emu-x", vec!["Starfall"]))
            .plugin(playable_games("emu-p", vec!["Lumen Gate"]))
            .plugin(games_on("emu-pc", "PC", vec![]).console("PC"))
            // The game catalog, for the games it offers.
            .built_in(nexus_api::PluginByKind::service(release_catalog::ReleaseCatalog::default()))
    });
    host.ui.start().unwrap();
    // The sample catalogs, fetched from the catalog address.
    let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugins/services/release-catalog/samples");
    for console in ["ps5", "ps4", "pc"] {
        let file = std::fs::read(samples.join(format!("{console}.json"))).unwrap();
        host.test.http.serve(&format!("https://xyz.dev/catalog/{console}.json"), file);
    }
    serve_steam_art(&host);
    // KytyPS5's builds: none yet, so its block shows no notice.
    let platform = host.core().kernel().host().platform;
    let (repo, _) = emu_kyty::release_source(&emu_kyty::default_settings(platform), platform).unwrap();
    host.test.http.serve(&format!("https://api.github.com/repos/{repo}/releases?per_page=10"), "[]");
    host.dispatch(Command::SetConfig {
        plugin: "release-catalog".parse().unwrap(),
        scope: nexus_api::ConfigScope::Emulator,
        value: serde_json::json!({ "catalog_url": "https://xyz.dev/catalog" }),
    });
    host.catalogs_fetched();
    host.dispatch(Command::RefreshLibrary);
    host.dispatch(Command::Select("emu-a:G1".parse().unwrap()));
    host.dispatch(Command::Search("a".into()));
    host
}

/// Fails unless each named picture in `out` has more than one colour.
fn not_blank(out: &Path, names: &[&str]) {
    for name in names {
        let png = image::open(out.join(format!("{name}.png"))).unwrap().to_rgba8();
        let first = png.get_pixel(0, 0);
        assert!(png.pixels().any(|p| p != first), "{name}: the screenshot is not blank");
    }
}

#[test]
fn every_default_screen_renders_the_library() {
    let (window, out) = offscreen();
    let home = tempfile::tempdir().unwrap();
    let mut host = library(home.path());
    shoot_default(&mut host, &window, &out, "default");
    shoot_sizes(&mut host, &window, &out);
    shoot_steam_offers(&mut host, &window, &out);
    // A first play asks before the emulator downloads.
    host.dispatch(Command::Launch("emu-x:G0".parse().unwrap()));
    shoot(&window, &out.join("default-install-prompt.png"));
    host.dispatch(Command::CancelInstall);
    // A game runs; the PS button brought the launcher back in front of it.
    host.dispatch(Command::Launch("emu-p:G0".parse().unwrap()));
    // The game watcher's rounds bring the game to the front.
    run_until(&host, 5000, |core| core.game_in_front());
    host.dispatch(Command::ToggleFocus);
    // The theme selects the running game: carry that out, as the app does.
    slint::platform::update_timers_and_animations();
    host.pump();
    host.load_images();
    shoot(&window, &out.join("default-running.png"));
    // The other stub emulators name no console, so no catalog is asked for them.
    assert_served(&host, &[]);
    println!("screenshots in {}", out.display());
    not_blank(&out, &["default-home", "default-library", "default-detail", "default-settings", "default-running"]);
}

#[test]
fn the_default_screens_render_right_to_left() {
    let (window, out) = offscreen();
    let home = tempfile::tempdir().unwrap();
    let mut host = library(home.path());
    host.dispatch(Command::SetThemeOption { key: "direction".into(), value: "rtl".into() });
    shoot_default(&mut host, &window, &out, "default-rtl");
    not_blank(&out, &["default-rtl-home"]);
}

#[test]
fn a_child_theme_renders_the_same_library() {
    let (window, out) = offscreen();
    let home = tempfile::tempdir().unwrap();
    let mut host = library(home.path());
    host.ui.switch(&id("fixture-grid")).unwrap();
    host.load_images();
    shoot(&window, &out.join("fixture-grid-home.png"));
    not_blank(&out, &["fixture-grid-home"]);
}
