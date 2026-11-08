//! Seam 3, composed host: a system's icon and cover. The emulator plugin that plays the system
//! ships them in its assets, and a theme can put its own in their place with
//! `[theme.systems.<SYSTEM>]`.

use crate::common::*;
use nexus_core::Command;
use slint_interpreter::Value;

/// A picture `width` by `height` in one color, as PNG bytes. The sizes tell the pictures apart.
fn png(width: u32, height: u32) -> Vec<u8> {
    let image = image::RgbImage::from_pixel(width, height, image::Rgb([40, 90, 160]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

/// The fields of each `Vm.emulators` entry, as the theme sees them, once their images are in.
fn emulators(host: &mut Composed) -> Vec<slint_interpreter::Struct> {
    host.dispatch(Command::Library(nexus_core::LibraryCmd::Refresh));
    host.load_images();
    let Value::Model(model) = host.instance().get_global_property("Vm", "emulators").unwrap() else {
        panic!("Vm.emulators is a list")
    };
    use slint::Model;
    model
        .iter()
        .map(|row| match row {
            Value::Struct(s) => s,
            other => panic!("an emulator is a struct, not {other:?}"),
        })
        .collect()
}

/// The size of the image in `field`, (0, 0) for none.
fn size(emulator: &slint_interpreter::Struct, field: &str) -> (u32, u32) {
    match emulator.get_field(field) {
        Some(Value::Image(image)) => (image.size().width, image.size().height),
        other => panic!("{field} is an image, not {other:?}"),
    }
}

fn text(emulator: &slint_interpreter::Struct, field: &str) -> String {
    match emulator.get_field(field) {
        Some(Value::String(s)) => s.to_string(),
        other => panic!("{field} is text, not {other:?}"),
    }
}

/// The emulator for `system` among `emulators`.
fn of<'a>(emulators: &'a [slint_interpreter::Struct], system: &str) -> &'a slint_interpreter::Struct {
    emulators.iter().find(|e| text(e, "system") == system).unwrap_or_else(|| panic!("no emulator for {system}"))
}

// The default theme draws no system art, so this also shows that every system's pictures load
// as soon as the view binds them, not when a theme first shows one.
#[test]
fn the_emulator_s_icon_and_cover_reach_the_theme_with_its_system() {
    let mut host = composed(|h| {
        h.plugin(FakeCatalog::new("emu-a").playable().system("PS4").icon(png(8, 8)).cover(png(32, 18)).build())
    });
    host.ui.start().unwrap();
    let shown = emulators(&mut host);
    let ps4 = of(&shown, "PS4");
    assert_eq!(size(ps4, "icon"), (8, 8));
    assert_eq!(size(ps4, "cover"), (32, 18));
    assert_eq!(text(ps4, "system-name"), "PlayStation 4");
}

#[test]
fn an_emulator_without_art_has_none() {
    let mut host = composed(|h| h.plugin(FakeCatalog::new("emu-a").playable().system("PS4").build()));
    host.ui.start().unwrap();
    let shown = emulators(&mut host);
    let ps4 = of(&shown, "PS4");
    assert_eq!(size(ps4, "icon"), (0, 0));
    assert_eq!(size(ps4, "cover"), (0, 0));
}

#[test]
fn each_real_emulator_shows_its_system_s_icon_and_cover() {
    use nexus_api::PluginBox;
    let mut host = composed(|h| {
        h.built_in(PluginBox::code(emu_kyty::KytyPlugin))
            .built_in(PluginBox::code(emu_shadps4::ShadPlugin))
            .built_in(PluginBox::code(emu_lichii::LichiiPlugin))
            .built_in(PluginBox::code(emu_pc::PcPlugin))
            .built_in(PluginBox::code(emu_steam::SteamPlugin))
    });
    host.ui.start().unwrap();
    let shown = emulators(&mut host);
    for system in ["PS5", "PS4", "SWITCH", "PC", "Steam"] {
        let emulator = of(&shown, system);
        assert_eq!(size(emulator, "icon"), (256, 256), "{system}'s icon");
        assert_eq!(size(emulator, "cover"), (1672, 941), "{system}'s cover");
    }
}

/// A theme `id` in `dir` that extends `parent`, with `art` as `[theme.systems.*]` lines and a
/// picture at each path it names: `(path, width, height)`.
fn art_theme(dir: &std::path::Path, id: &str, parent: &str, art: &str, pictures: &[(&str, u32, u32)]) {
    write_theme(dir, id, &format!("extends = \"{parent}\"\n{art}"), &[]);
    for (path, width, height) in pictures {
        let file = dir.join(id).join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, png(*width, *height)).unwrap();
    }
}

fn ps4_with_art(h: nexus_testkit::FakeHost) -> nexus_testkit::FakeHost {
    h.plugin(FakeCatalog::new("emu-a").playable().system("PS4").icon(png(8, 8)).cover(png(32, 18)).build())
}

#[test]
fn a_theme_s_cover_for_a_system_takes_the_place_of_the_plugin_s_and_keeps_its_icon() {
    let dir = tempfile::tempdir().unwrap();
    art_theme(
        dir.path(),
        "arty",
        "default",
        "[theme.systems.PS4]\ncover = \"art/ps4.png\"",
        &[("art/ps4.png", 64, 36)],
    );
    let mut host = composed(|h| ps4_with_art(h.discover(dir.path())));
    host.ui.start().unwrap();
    host.ui.switch(&nexus_api::PluginId::new("arty").unwrap()).unwrap();
    let shown = emulators(&mut host);
    let ps4 = of(&shown, "PS4");
    assert_eq!(size(ps4, "cover"), (64, 36), "the theme's");
    assert_eq!(size(ps4, "icon"), (8, 8), "the plugin's");
}

#[test]
fn a_child_theme_s_system_art_wins_over_its_parent_s_per_picture() {
    let dir = tempfile::tempdir().unwrap();
    art_theme(
        dir.path(),
        "parent-art",
        "default",
        "[theme.systems.PS4]\nicon = \"art/icon.png\"\ncover = \"art/cover.png\"",
        &[("art/icon.png", 10, 10), ("art/cover.png", 40, 20)],
    );
    art_theme(
        dir.path(),
        "child-art",
        "parent-art",
        "[theme.systems.PS4]\nicon = \"mine.png\"",
        &[("mine.png", 12, 12)],
    );
    let mut host = composed(|h| ps4_with_art(h.discover(dir.path())));
    host.ui.start().unwrap();
    host.ui.switch(&nexus_api::PluginId::new("child-art").unwrap()).unwrap();
    let shown = emulators(&mut host);
    let ps4 = of(&shown, "PS4");
    assert_eq!(size(ps4, "icon"), (12, 12), "the child's");
    assert_eq!(size(ps4, "cover"), (40, 20), "the parent's");
}

#[test]
fn a_theme_s_art_for_a_system_no_emulator_plays_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    art_theme(
        dir.path(),
        "arty",
        "default",
        "[theme.systems.SWITCH]\ncover = \"art/switch.png\"",
        &[("art/switch.png", 64, 36)],
    );
    let mut host = composed(|h| ps4_with_art(h.discover(dir.path())));
    host.ui.start().unwrap();
    host.ui.switch(&nexus_api::PluginId::new("arty").unwrap()).unwrap();
    let shown = emulators(&mut host);
    assert_eq!(size(of(&shown, "PS4"), "cover"), (32, 18));
}

#[test]
fn a_theme_whose_system_picture_is_missing_does_not_load_and_the_old_theme_stays() {
    let dir = tempfile::tempdir().unwrap();
    art_theme(dir.path(), "arty", "default", "[theme.systems.PS4]\ncover = \"art/gone.png\"", &[]);
    let mut host = composed(|h| ps4_with_art(h.discover(dir.path())));
    host.ui.start().unwrap();
    let error = host.ui.switch(&nexus_api::PluginId::new("arty").unwrap()).unwrap_err();
    assert!(error.to_string().contains("art/gone.png"), "{error}");
    assert_eq!(host.ui.current().map(|id| id.as_str()), Some("default"));
}

#[test]
fn a_theme_whose_system_picture_does_not_decode_does_not_load_and_the_old_theme_stays() {
    let dir = tempfile::tempdir().unwrap();
    write_theme(
        dir.path(),
        "arty",
        "extends = \"default\"\n[theme.systems.PS4]\ncover = \"art/bad.png\"",
        &[("art/bad.png", "not a picture")],
    );
    let mut host = composed(|h| ps4_with_art(h.discover(dir.path())));
    host.ui.start().unwrap();
    let error = host.ui.switch(&nexus_api::PluginId::new("arty").unwrap()).unwrap_err();
    assert!(error.to_string().contains("systems.PS4.cover"), "{error}");
    assert_eq!(host.ui.current().map(|id| id.as_str()), Some("default"));
}
