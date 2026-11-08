//! The Default theme's files: its animation times and its pictures.

use crate::classic_theme::theme_files::times_outside_look_timed;

fn theme_folder() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugins/themes/default/theme")
}

#[test]
fn every_animation_time_goes_through_look_timed_so_animations_off_cuts_it() {
    let theme = theme_folder();
    let mut files = Vec::new();
    for dir in ["screens", "components"] {
        for entry in std::fs::read_dir(theme.join(dir)).unwrap() {
            files.push(entry.unwrap().path());
        }
    }
    let wrong = times_outside_look_timed(&theme, &files);
    assert!(wrong.is_empty(), "times outside Look.timed:\n{}", wrong.join("\n"));
}

#[test]
fn the_all_games_pictures_ship_at_a_size_the_host_shows_whole() {
    let theme = theme_folder();
    let size = |path: &str| image::image_dimensions(theme.join(path)).unwrap_or_else(|e| panic!("{path}: {e}"));
    assert_eq!(size("images/all-games-icon.png"), (256, 256));
    let (width, height) = size("images/all-games-cover.jpg");
    assert!(width <= nexus_ui_host::MAX_WIDTH && height <= nexus_ui_host::MAX_HEIGHT, "{width}×{height}");
    // 16:9, as the screen it fills: no side of it is cut away.
    assert_eq!(width * 9 / 16, height, "{width}×{height}");
}
