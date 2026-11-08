//! The Systems theme's files: its animation times and its pictures.

use crate::default_theme::theme_files::{fixed_times, without_timed};

fn theme_folder() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugins/themes/systems/theme")
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
    let mut wrong = Vec::new();
    for file in &files {
        let source = std::fs::read_to_string(file).unwrap();
        for (number, line) in source.lines().enumerate() {
            for field in ["duration:", "delay:"] {
                let Some(at) = line.find(field) else { continue };
                let value = &line[at + field.len()..];
                let value = &value[..value.find([';', '}']).unwrap_or(value.len())];
                if !fixed_times(&without_timed(value)).is_empty() {
                    let name = file.strip_prefix(&theme).unwrap().display();
                    wrong.push(format!("{name}:{}: {}", number + 1, value.trim()));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "times outside Look.timed:\n{}", wrong.join("\n"));
}

#[test]
fn the_all_games_pictures_ship_at_a_size_the_host_shows_whole() {
    let theme = theme_folder();
    let size = |path: &str| image::image_dimensions(theme.join(path)).unwrap_or_else(|e| panic!("{path}: {e}"));
    assert_eq!(size("images/all-games-icon.png"), (256, 256));
    let (width, height) = size("images/all-games-cover.jpg");
    assert!(width <= nexus_ui_host::MAX_WIDTH && height <= nexus_ui_host::MAX_HEIGHT, "{width}×{height}");
    assert_eq!((width, height), (1672, 941), "the systems' covers' size, so they line up");
}
