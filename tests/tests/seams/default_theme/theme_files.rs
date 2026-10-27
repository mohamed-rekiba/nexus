//! The theme's files: the fonts it ships and the animation times it uses.

/// `text` without its `Look.timed(...)` calls.
fn without_timed(text: &str) -> String {
    let mut rest = text;
    let mut out = String::new();
    while let Some(at) = rest.find("Look.timed(") {
        out.push_str(&rest[..at]);
        let mut depth = 0;
        let mut end = rest.len();
        for (i, c) in rest[at..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' if depth == 1 => {
                    end = at + i + 1;
                    break;
                }
                ')' => depth -= 1,
                _ => {}
            }
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// The times in `text` that are not 0, such as "40ms" or "6s".
fn fixed_times(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let after_name = i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '-' || chars[i - 1] == '_');
        if !chars[i].is_ascii_digit() || after_name {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
            i += 1;
        }
        let number: String = chars[start..i].iter().collect();
        let unit: String = chars[i..].iter().take_while(|c| c.is_ascii_alphabetic()).collect();
        if (unit == "ms" || unit == "s") && number.parse::<f64>().is_ok_and(|n| n != 0.0) {
            found.push(format!("{number}{unit}"));
        }
        i += unit.len();
    }
    found
}

/// The default theme's folder, and its .slint files.
fn theme_sources() -> (std::path::PathBuf, Vec<std::path::PathBuf>) {
    fn slint_files(dir: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                slint_files(&path, files);
            } else if path.extension().is_some_and(|e| e == "slint") {
                files.push(path);
            }
        }
    }
    let theme = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugins/themes/default/theme");
    let mut files = Vec::new();
    slint_files(&theme, &mut files);
    assert!(files.len() > 10, "the theme's files: {files:?}");
    (theme, files)
}

#[test]
fn every_font_the_theme_names_ships_with_it() {
    let (theme, files) = theme_sources();
    let shipped: Vec<String> = std::fs::read_dir(theme.join("fonts"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_lowercase())
        .collect();
    let mut wrong = Vec::new();
    for file in &files {
        let source = std::fs::read_to_string(file).unwrap();
        for (number, line) in source.lines().enumerate() {
            let Some(at) = line.find("font-family:") else { continue };
            let parts: Vec<&str> = line[at..].split('"').collect();
            // Every other part is quoted; one after == or != is a value compared, not a font.
            for pair in parts.chunks(2).filter(|p| p.len() == 2) {
                let (before, name) = (pair[0].trim_end(), pair[1]);
                if before.ends_with("==") || before.ends_with("!=") {
                    continue;
                }
                let file_name = name.replace(' ', "").to_lowercase();
                if !shipped.iter().any(|f| f.starts_with(&file_name)) {
                    wrong.push(format!("{}:{}: {name}", file.strip_prefix(&theme).unwrap().display(), number + 1));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "fonts the theme doesn't ship fall back to whatever the system has: {wrong:?}");
}

#[test]
fn every_animation_time_goes_through_look_timed_so_animations_off_cuts_it() {
    let (theme, files) = theme_sources();
    // With Animations off the motion tokens are 0, so "Tokens.motion-normal - 20ms" is -20 ms
    // and a bare "300ms" stays 300 ms. `Look.timed` makes both 0.
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
