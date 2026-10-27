//! The command line: its options, and the folders they default to.

use std::path::PathBuf;

use nexus_api::PluginId;

const USAGE: &str = "\
nexus [options]

  --theme <id>        Show this theme now (not saved)
  --safe-mode         Load built-in plugins and the default theme only
  --dev               Reload theme files when they change
  --windowed          Open in a normal window for this run (full screen is the default)
  --monitor <name>    Open on this monitor for this run, such as DP-2 (not saved; a screen
                      picked in Settings wins)
  --data-dir <path>   Where settings and plugin data live
  --extensions <path> Folder of third-party plugins and themes (default: <data-dir>/extensions)
  --folder <name>=<path>
                      Map a named host folder, such as games=/srv/games (saved; repeat
                      for more folders under one name)
  --supports <feature>
                      Exit 0 when this launcher has the feature (gamescope,
                      system-settings, os-install), else 1
  --version           Show the version, on one line
  --help              Show this help

Environment:
  NEXUS_SCALE=<n>     Draw everything at this size for this run, from 0.8 to 1.4 (overrides
                      Size of everything in Settings)
";

pub struct Args {
    pub theme: Option<PluginId>,
    pub safe_mode: bool,
    pub dev: bool,
    pub windowed: bool,
    pub monitor: Option<String>,
    pub data_dir: PathBuf,
    pub extensions: Option<PathBuf>,
    pub folders: std::collections::BTreeMap<String, Vec<PathBuf>>,
}

pub fn home_dir() -> Option<PathBuf> {
    let name = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(name).map(PathBuf::from)
}

pub fn default_data_dir() -> PathBuf {
    let home = home_dir().unwrap_or_else(|| PathBuf::from("."));
    if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| home.join("AppData/Roaming")).join("nexus")
    } else if cfg!(target_os = "macos") {
        home.join("Library/Application Support/nexus")
    } else {
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share")).join("nexus")
    }
}

/// Themes and plugins that ship with the launcher: `extensions` next to the binary (and, in a
/// macOS app bundle, in its `Resources`). A packaged build puts the example themes there.
pub fn bundled_extensions() -> PathBuf {
    let Ok(exe) = std::env::current_exe() else { return PathBuf::from("extensions") };
    let dir = exe.parent().map(PathBuf::from).unwrap_or_default();
    let resources = dir.join("../Resources/extensions");
    if resources.is_dir() { resources } else { dir.join("extensions") }
}

/// What `--version` prints: one line, which Nexus OS keeps in its release marker.
pub fn version_line() -> String {
    format!("nexus {}", env!("CARGO_PKG_VERSION"))
}

/// Whether this launcher has `feature`, for `--supports` (docs/nexus-os.md):
/// - `gamescope`: it runs in a Gamescope session and sets the window ids Gamescope shows;
/// - `system-settings`: Settings → System for Nexus OS;
/// - `os-install`: the stick's welcome and install screens, which the stick's build checks.
///
/// A launcher that doesn't know the option exits with an error, so a session can tell the two
/// apart.
pub fn supports(feature: &str) -> bool {
    matches!(feature, "gamescope" | "system-settings" | "os-install")
}

/// The size of everything from `NEXUS_SCALE`, for this run: a number, which the theme host
/// keeps between 0.8 and 1.4. `None` when it isn't set or isn't a number.
pub fn size_override(value: Option<&str>) -> Option<f32> {
    value?.trim().parse::<f32>().ok().filter(|size| size.is_finite())
}

pub fn parse_args(mut it: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut args = Args {
        theme: None,
        safe_mode: false,
        dev: false,
        windowed: false,
        monitor: None,
        data_dir: default_data_dir(),
        extensions: None,
        folders: Default::default(),
    };
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--theme" => args.theme = Some(PluginId::new(value("--theme")?).map_err(|e| e.to_string())?),
            "--safe-mode" => args.safe_mode = true,
            "--dev" => args.dev = true,
            "--windowed" => args.windowed = true,
            "--monitor" => args.monitor = Some(value("--monitor")?),
            "--data-dir" => args.data_dir = PathBuf::from(value("--data-dir")?),
            "--extensions" => args.extensions = Some(PathBuf::from(value("--extensions")?)),
            "--folder" => {
                let mapping = value("--folder")?;
                let (name, path) = mapping.split_once('=').ok_or("--folder needs <name>=<path>")?;
                args.folders.entry(name.to_owned()).or_default().push(PathBuf::from(path));
            }
            "--help" | "-h" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "--version" | "-V" => {
                println!("{}", version_line());
                std::process::exit(0);
            }
            other => return Err(format!("unknown option {other}\n\n{USAGE}")),
        }
    }
    Ok(args)
}

#[cfg(test)]
mod tests {
    fn parse(args: &[&str]) -> Result<super::Args, String> {
        super::parse_args(args.iter().map(|a| a.to_string()))
    }

    #[test]
    fn monitor_takes_a_name_for_this_run() {
        assert_eq!(parse(&["--monitor", "DELL U2720Q"]).unwrap().monitor.as_deref(), Some("DELL U2720Q"));
        assert_eq!(parse(&["--windowed"]).unwrap().monitor, None);
        assert_eq!(parse(&["--monitor"]).err().as_deref(), Some("--monitor needs a value"));
        assert!(parse(&["--screen", "x"]).err().unwrap_or_default().starts_with("unknown option --screen"));
    }

    #[test]
    fn nexus_scale_sets_the_size_when_it_is_a_number() {
        assert_eq!(super::size_override(Some("1.25")), Some(1.25));
        assert_eq!(super::size_override(Some(" 0.9 ")), Some(0.9));
        assert_eq!(super::size_override(Some("big")), None);
        assert_eq!(super::size_override(Some("NaN")), None);
        assert_eq!(super::size_override(None), None);
    }

    #[test]
    fn version_is_one_line_with_the_release_version() {
        let line = super::version_line();
        assert!(!line.contains('\n'), "{line:?}");
        let workspace = include_str!("../../../Cargo.toml");
        let version = workspace.lines().find_map(|l| l.strip_prefix("version = \"")?.strip_suffix('"')).unwrap();
        assert_eq!(line, format!("nexus {version}"));
    }

    #[test]
    fn the_launcher_supports_gamescope_the_os_s_settings_and_its_install() {
        assert!(super::supports("gamescope"));
        assert!(super::supports("system-settings"));
        assert!(super::supports("os-install"), "the stick's build checks it");
        assert!(!super::supports("wayland"));
        assert!(!super::supports(""));
    }
}
