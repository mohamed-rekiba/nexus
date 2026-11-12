//! The Lichii config: what Nexus sets in Lichii's own settings file, `qt-config.ini`, which
//! Lichii's window saves. A game Nexus starts with an official build runs in eden-cli with a
//! settings file (`eden-cli -c <file>`): Lichii's own settings file with Nexus's overrides over
//! it (airplane mode) and players 1 to 4 on the controllers the computer has.
//!
//! Your own build and Lichii's window have no such file: they read `qt-config.ini` alone, so
//! Nexus's settings don't reach them.
//!
//! No `qt-config.ini` (Lichii's window never saved one) is fine: Lichii's defaults apply under
//! Nexus's settings. One that is there but doesn't read stops the game with the reason, because
//! a game started without it would run on Lichii's defaults without a word.
//!
//! Lichii (like Eden and yuzu) names a controller by its SDL GUID and port, and each Switch
//! button by the SDL input that gives it: `engine:sdl,port:0,guid:<guid>,button:1`. The Switch
//! button comes from SDL's gamepad layout as Lichii's `SDLDriver::GetDefaultButtonBinding`
//! pairs them: A is the east button, B the south one, X north and Y west, ZL and ZR the
//! triggers. The SDL input comes from the controller's SDL mapping (`b:b1,dpup:h0.1,...`).
//!
//! A player whose controls the player set in Lichii (any player 1 to 4 on something other than
//! the keyboard) keeps them: then the controls stay as they are.
//!
//! Lichii's own default is airplane mode on, which blocks every TCP socket a game opens, so an
//! online game shows "no internet connection". Nexus's setting is off by default. An empty
//! `network_interface` is fine: Lichii then uses the first interface it finds.

use std::collections::HashMap;

use emulator_kit::host::Host;
use nexus_api::{Controller, HostPath, MAX_FOLDER_READ, PluginError, PluginResult, RelPath};

use crate::{CONFIG, Settings};

/// Lichii's own settings file, which its window saves, in Lichii's settings folder.
pub const QT_CONFIG: &str = "qt-config.ini";
/// The settings file games start with, in the plugin's folder.
const GAME_SETTINGS: &str = "eden-cli/config.ini";

/// The section Lichii keeps the controls in.
const CONTROLS: &str = "Controls";
/// The section Lichii keeps airplane mode in.
const SERVICES: &str = "Services";
/// The players Nexus sets.
const PLAYERS: usize = 4;

/// Each Switch button's key in Lichii's own settings file, and the SDL gamepad input that gives
/// it, as SDL's mapping strings name it (`a` is the south button, `b` the east one).
const BUTTONS: [(&str, &str); 22] = [
    ("button_a", "b"),
    ("button_b", "a"),
    ("button_x", "y"),
    ("button_y", "x"),
    ("button_lstick", "leftstick"),
    ("button_rstick", "rightstick"),
    ("button_l", "leftshoulder"),
    ("button_r", "rightshoulder"),
    ("button_zl", "lefttrigger"),
    ("button_zr", "righttrigger"),
    ("button_plus", "start"),
    ("button_minus", "back"),
    ("button_dleft", "dpleft"),
    ("button_dup", "dpup"),
    ("button_dright", "dpright"),
    ("button_ddown", "dpdown"),
    ("button_slleft", "leftshoulder"),
    ("button_srleft", "rightshoulder"),
    ("button_home", "guide"),
    ("button_screenshot", "misc1"),
    ("button_slright", "leftshoulder"),
    ("button_srright", "rightshoulder"),
];

/// Each stick's key, and its SDL axes.
const STICKS: [(&str, &str, &str); 2] = [("lstick", "leftx", "lefty"), ("rstick", "rightx", "righty")];

/// Writes the settings file for a game Nexus starts with an official build, and returns where
/// it is: Lichii's own settings file with Nexus's overrides from `settings` over it and players
/// 1 to 4 on `controllers`. Fails when Lichii's own settings file is there but doesn't read.
pub async fn write(host: &Host, controllers: &[Controller], settings: &Settings) -> PluginResult<RelPath> {
    let base = read_qt_config(host).await?;
    let path = RelPath::new(GAME_SETTINGS)?;
    host.storage.write(&path, settings_file(&base, controllers, settings).as_bytes()).await?;
    Ok(path)
}

/// Lichii's own settings file: empty when Lichii's settings folder isn't set or the file isn't
/// there, so Lichii's defaults apply.
async fn read_qt_config(host: &Host) -> PluginResult<String> {
    let unread =
        |why: String| PluginError::Storage(format!("Lichii's own settings file ({QT_CONFIG}) doesn't read: {why}"));
    let roots = host.folders.roots(CONFIG).await.map_err(|e| unread(e.to_string()))?;
    let Some(root) = roots.first().map(|r| r.root) else { return Ok(String::new()) };
    let file = HostPath::new(CONFIG, root, RelPath::new(QT_CONFIG)?);
    if !host.folders.exists(&file).await.map_err(|e| unread(e.to_string()))? {
        return Ok(String::new());
    }
    let bytes = host.folders.read_range(&file, 0, MAX_FOLDER_READ).await.map_err(|e| unread(e.to_string()))?;
    // A read stops at the limit, so a file of the limit's size may be bigger.
    if bytes.len() as u64 >= MAX_FOLDER_READ {
        return Err(unread(format!("the file is {MAX_FOLDER_READ} bytes or more")));
    }
    String::from_utf8(bytes).map_err(|_| unread("the file isn't text".into()))
}

/// The settings file for `eden-cli`: `base` (Lichii's own settings file, or empty) with Nexus's
/// overrides from `settings` and players 1 to 4 on `controllers`, in order. The controls stay
/// as they are when there is no controller, or when the player set a controller in Lichii.
fn settings_file(base: &str, controllers: &[Controller], settings: &Settings) -> String {
    let mut ini = Ini::parse(base);
    // Nexus's one override. It is written with `\default=false`, so Lichii reads the value.
    ini.set(SERVICES, "airplane_mode", &settings.airplane_mode.to_string());
    if controllers.is_empty() || ini.has_own_controllers() {
        return ini.text();
    }
    let assigned = controllers.iter().take(PLAYERS).enumerate();
    let mut lines = Vec::new();
    for (player, controller) in assigned.clone() {
        lines.extend(player_lines(player, controller));
    }
    let players: Vec<usize> = assigned.map(|(player, _)| player).collect();
    ini.replace_controls(&players, lines);
    ini.text()
}

/// The lines for `player` (0 to 3) on `controller`.
fn player_lines(player: usize, controller: &Controller) -> Vec<String> {
    let inputs: HashMap<&str, &str> = controller.mapping.split(',').filter_map(|pair| pair.split_once(':')).collect();
    let device = format!("engine:sdl,port:{},guid:{}", controller.port, controller.guid);
    let mut lines =
        vec![format!("player_{player}_connected\\default=false"), format!("player_{player}_connected=true")];
    let mut set = |key: &str, value: String| {
        lines.push(format!("player_{player}_{key}\\default=false"));
        lines.push(format!("player_{player}_{key}=\"{value}\""));
    };
    for (key, input) in BUTTONS {
        if let Some(binding) = inputs.get(input).and_then(|b| button(b)) {
            set(key, format!("{device},{binding}"));
        }
    }
    for (key, x, y) in STICKS {
        let (Some(x), Some(y)) = (inputs.get(x).and_then(|a| axis(a)), inputs.get(y).and_then(|a| axis(a))) else {
            continue;
        };
        let stick = format!("axis_x:{x},axis_y:{y},offset_x:0.000000,offset_y:0.000000,invert_x:+,invert_y:+");
        set(key, format!("{device},{stick}"));
    }
    lines
}

/// The axis number of an SDL input such as `a2`, `+a2`, `-a2` or `a2~`.
fn axis(input: &str) -> Option<u32> {
    input.trim_start_matches(['+', '-']).trim_end_matches('~').strip_prefix('a')?.parse().ok()
}

/// Lichii's words for the SDL input that gives a button: `b1` is `button:1`, `h0.1` is
/// `hat:0,direction:up`, and an axis (a trigger) is pressed past half way.
fn button(input: &str) -> Option<String> {
    if let Some(n) = input.strip_prefix('b') {
        return n.parse::<u32>().ok().map(|n| format!("button:{n}"));
    }
    if let Some(hat) = input.strip_prefix('h') {
        let (hat, mask) = hat.split_once('.')?;
        let direction = match mask {
            "1" => "up",
            "2" => "right",
            "4" => "down",
            "8" => "left",
            _ => return None,
        };
        return Some(format!("hat:{},direction:{direction}", hat.parse::<u32>().ok()?));
    }
    axis(input).map(|n| format!("axis:{n},threshold:0.5,invert:+"))
}

/// An INI file as sections of lines, in order, kept as they are but for the controls Nexus
/// sets.
struct Ini {
    /// The lines before the first section, then each section: its name and lines.
    sections: Vec<(Option<String>, Vec<String>)>,
}

impl Ini {
    fn parse(text: &str) -> Self {
        let mut sections = vec![(None, Vec::new())];
        for line in text.lines() {
            let name = line.trim().strip_prefix('[').and_then(|l| l.strip_suffix(']'));
            match name {
                Some(name) => sections.push((Some(name.to_owned()), Vec::new())),
                None => sections.last_mut().expect("one section at least").1.push(line.to_owned()),
            }
        }
        Self { sections }
    }

    fn controls(&self) -> Option<&Vec<String>> {
        self.sections.iter().find(|(name, _)| name.as_deref() == Some(CONTROLS)).map(|(_, lines)| lines)
    }

    /// The player set a controller for one of players 1 to 4 in Lichii: a button of theirs is
    /// set, not left to its default, and isn't on the keyboard.
    fn has_own_controllers(&self) -> bool {
        let Some(lines) = self.controls() else { return false };
        let value = |key: &str| {
            lines.iter().find_map(|l| l.split_once('=').filter(|(k, _)| k.trim() == key).map(|(_, v)| v.trim()))
        };
        (0..PLAYERS).any(|player| {
            BUTTONS.iter().any(|(button, _)| {
                let key = format!("player_{player}_{button}");
                value(&format!("{key}\\default")) == Some("false")
                    && value(&key).is_some_and(|v| !v.is_empty() && !v.contains("engine:keyboard"))
            })
        })
    }

    /// The controls of `players` become `lines`: their old lines go, the new ones end the
    /// controls section, which is added when there is none.
    fn replace_controls(&mut self, players: &[usize], lines: Vec<String>) {
        let set_keys: Vec<String> = lines.iter().filter_map(|l| l.split_once('=')).map(|(k, _)| k.to_owned()).collect();
        let ours = |line: &str| {
            let key = line.split_once('=').map(|(k, _)| k.trim()).unwrap_or("");
            let key = key.strip_suffix("\\default").unwrap_or(key);
            players.iter().any(|p| {
                let prefix = format!("player_{p}_");
                key.strip_prefix(&prefix).is_some_and(|rest| {
                    rest == "connected"
                        || BUTTONS.iter().any(|(b, _)| *b == rest)
                        || STICKS.iter().any(|(s, ..)| *s == rest)
                })
            }) || set_keys.iter().any(|k| k == key)
        };
        let section = self.section_mut(CONTROLS);
        section.retain(|line| !ours(line));
        end_section(section, lines);
    }

    /// `key` in `section` becomes `value`, and not its default: its old lines go, the new ones
    /// end the section.
    fn set(&mut self, section: &str, key: &str, value: &str) {
        let default = format!("{key}\\default");
        let section = self.section_mut(section);
        section.retain(|line| line.split_once('=').is_none_or(|(k, _)| k.trim() != key && k.trim() != default));
        end_section(section, vec![format!("{default}=false"), format!("{key}={value}")]);
    }

    /// The lines of section `name`, which is added at the end when there is none.
    fn section_mut(&mut self, name: &str) -> &mut Vec<String> {
        let at = match self.sections.iter().position(|(n, _)| n.as_deref() == Some(name)) {
            Some(at) => at,
            None => {
                self.sections.push((Some(name.to_owned()), Vec::new()));
                self.sections.len() - 1
            }
        };
        &mut self.sections[at].1
    }

    fn text(&self) -> String {
        let mut out = String::new();
        for (name, lines) in &self.sections {
            if let Some(name) = name {
                out.push_str(&format!("[{name}]\n"));
            }
            for line in lines {
                out.push_str(line);
                out.push('\n');
            }
        }
        out
    }
}

/// `lines` end `section`, before the blank lines that end it.
fn end_section(section: &mut Vec<String>, lines: Vec<String>) {
    let end = section.iter().rposition(|l| !l.trim().is_empty()).map_or(0, |i| i + 1);
    section.splice(end..end, lines);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What Nexus adds to a file with no `[Services]` section.
    const AIRPLANE_OFF: &str = "[Services]\nairplane_mode\\default=false\nairplane_mode=false\n";

    fn airplane(on: bool) -> Settings {
        Settings { airplane_mode: on, ..Settings::default() }
    }

    fn pad(port: u32, mapping: &str) -> Controller {
        Controller { name: "Pad".into(), guid: "0300".into(), port, mapping: mapping.into() }
    }

    #[test]
    fn sdl_inputs_read_as_lichii_writes_them() {
        assert_eq!(button("b10").as_deref(), Some("button:10"));
        assert_eq!(button("h0.8").as_deref(), Some("hat:0,direction:left"));
        assert_eq!(button("+a5").as_deref(), Some("axis:5,threshold:0.5,invert:+"));
        assert_eq!(button("h0.3"), None, "two directions at once is no button");
        assert_eq!(axis("a3~"), Some(3));
    }

    #[test]
    fn no_controller_or_the_player_s_own_controls_leave_the_controls_as_they_are() {
        let base = "[Controls]\nplayer_0_button_a\\default=true\nplayer_0_button_a=\"engine:keyboard,code:67\"\n";
        assert_eq!(settings_file(base, &[], &airplane(false)), format!("{base}{AIRPLANE_OFF}"));
        let own =
            "[Controls]\nplayer_1_button_b\\default=false\nplayer_1_button_b=\"engine:sdl,port:0,guid:ab,button:0\"\n";
        assert_eq!(
            settings_file(own, &[pad(0, "a:b0")], &airplane(false)),
            format!("{own}{AIRPLANE_OFF}"),
            "the player mapped a controller in Lichii"
        );
    }

    #[test]
    fn airplane_mode_is_nexus_s_and_the_other_services_stay() {
        let base = "[Services]\nnetwork_interface\\default=true\nnetwork_interface=\n\
                    airplane_mode\\default=true\nairplane_mode=true\nlan_play=false\n\n[UI]\ntheme=dark\n";
        for on in [false, true] {
            assert_eq!(
                settings_file(base, &[], &airplane(on)),
                format!(
                    "[Services]\nnetwork_interface\\default=true\nnetwork_interface=\nlan_play=false\n\
                     airplane_mode\\default=false\nairplane_mode={on}\n\n[UI]\ntheme=dark\n"
                )
            );
        }
    }

    #[test]
    fn a_file_without_controls_gets_them_and_the_other_sections_stay() {
        let out =
            settings_file("[Renderer]\nbackend=1\n\n[UI]\ntheme=dark\n", &[pad(0, "a:b0,b:b1")], &airplane(false));
        assert_eq!(
            out,
            "[Renderer]\nbackend=1\n\n[UI]\ntheme=dark\n\
             [Services]\nairplane_mode\\default=false\nairplane_mode=false\n[Controls]\n\
             player_0_connected\\default=false\nplayer_0_connected=true\n\
             player_0_button_a\\default=false\nplayer_0_button_a=\"engine:sdl,port:0,guid:0300,button:1\"\n\
             player_0_button_b\\default=false\nplayer_0_button_b=\"engine:sdl,port:0,guid:0300,button:0\"\n"
        );
    }
}
