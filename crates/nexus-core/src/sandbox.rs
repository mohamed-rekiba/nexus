//! A game in a sandbox ([`nexus_api::Sandbox`]): on Linux, through bubblewrap (`bwrap`).
//!
//! The game starts in a new, empty root. bubblewrap puts back only what a game needs:
//! - the system's programs, libraries and settings (`/usr`, `/etc`, `/opt`), read only;
//! - `/sys`, `/proc` and every device (the GPU, controllers, `ntsync`), with an empty
//!   `/dev/shm`: other programs' shared memory stays out;
//! - the display and the sound: the X11 sockets, and the Wayland, PipeWire and PulseAudio
//!   sockets of the player's runtime folder ([`Desktop`]);
//! - an empty home folder and an empty `/tmp`, which go when the game ends;
//! - the folders the plugin asked for, and nothing else of the player's files.
//!
//! The session D-Bus stays out: the game's bus is [`NO_BUS`]. Through the real one a program
//! can ask the player's systemd to start another program outside the sandbox. GameMode talks to
//! its daemon over it, so a sandboxed game runs without GameMode.
//!
//! The game keeps the system's process ids: Nexus finds its windows by them (`_NET_WM_PID`)
//! and follows its processes. It still can't read other programs' files, memory or
//! environment through `/proc`, because bubblewrap always runs it in a user namespace of its
//! own. The network stays open: a game may need it, and umu-launcher downloads Proton.

use std::path::{Path, PathBuf};

use crate::SpawnCommand;

/// What the desktop the game shows on has, found when Nexus starts ([`Desktop::detect`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Desktop {
    /// `XDG_RUNTIME_DIR`: the player's runtime folder. The game gets an empty one at the same
    /// path, with only the sockets below in it.
    pub runtime_dir: Option<PathBuf>,
    /// The sockets of the display and the sound, each at its own path.
    pub sockets: Vec<PathBuf>,
    /// Files the display needs: `XAUTHORITY`, the X11 server's key.
    pub files: Vec<PathBuf>,
}

impl Desktop {
    /// The desktop Nexus runs on, from its environment.
    pub fn detect() -> Self {
        let var = |name| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
        let runtime_dir = var("XDG_RUNTIME_DIR");
        let mut sockets = Vec::new();
        if let Some(dir) = &runtime_dir {
            // A relative WAYLAND_DISPLAY is in the runtime folder; an absolute one is a path.
            if let Some(wayland) = var("WAYLAND_DISPLAY") {
                sockets.push(dir.join(wayland));
            }
            sockets.push(dir.join("pipewire-0"));
            sockets.push(dir.join("pulse/native"));
        }
        Self { runtime_dir, sockets, files: var("XAUTHORITY").into_iter().collect() }
    }
}

/// The path of the session D-Bus a sandboxed game gets (`DBUS_SESSION_BUS_ADDRESS` is
/// `unix:path=` it): `/dev/null`, which no program can talk to, so each try fails at once.
/// - The variable can't be left unset: without it libdbus "autolaunches" a new `dbus-launch`
///   at each try, which stays running, and a game that keeps trying (Proton does) fills the
///   process table.
/// - The path must be there: Proton's own container (pressure-vessel) binds the bus's socket
///   into itself, and stops when it can't.
pub const NO_BUS: &str = "/run/nexus-sandbox/no-session-bus";

/// The system's folders a game reads, where the system has them.
const SYSTEM: [&str; 9] = ["/usr", "/bin", "/sbin", "/lib", "/lib64", "/lib32", "/etc", "/opt", "/sys"];
/// What the system needs from `/run`: the controllers' list (udev), and the name servers
/// `/etc/resolv.conf` may point to.
const RUN: [&str; 4] = ["/run/udev", "/run/systemd/resolve", "/run/NetworkManager", "/run/resolvconf"];

/// Adds `how <path> <path>`: `path` at the same path in the sandbox.
fn bind(args: &mut Vec<String>, how: &str, path: &Path) {
    let path = path.display().to_string();
    args.extend([how.to_owned(), path.clone(), path]);
}

/// `command` run through `bwrap`, in a sandbox that reads `read`, reads and writes `write`, and
/// has an empty `home`.
///
/// bubblewrap works through its words in order, and a later one goes on top of an earlier one.
/// So the empty folders come first, then the sockets and the folders asked for: a game folder
/// in `~/Games` shows inside the empty home.
pub(crate) fn wrap(
    command: SpawnCommand,
    bwrap: &Path,
    read: &[PathBuf],
    write: &[PathBuf],
    home: Option<&Path>,
    desktop: &Desktop,
) -> SpawnCommand {
    let mut args = Vec::new();
    for path in SYSTEM.iter().chain(&RUN) {
        bind(&mut args, "--ro-bind-try", Path::new(path));
    }
    args.extend(
        [
            "--dev-bind",
            "/dev",
            "/dev",
            "--tmpfs",
            "/dev/shm",
            "--proc",
            "/proc",
            "--tmpfs",
            "/tmp",
            "--dir",
            "/var/tmp",
        ]
        .map(String::from),
    );
    bind(&mut args, "--ro-bind-try", Path::new("/tmp/.X11-unix"));
    for dir in home.into_iter().chain(desktop.runtime_dir.as_deref()) {
        args.extend(["--perms", "0700", "--dir"].map(String::from));
        args.push(dir.display().to_string());
    }
    for path in desktop.sockets.iter().chain(&desktop.files) {
        bind(&mut args, "--ro-bind-try", path);
    }
    for path in read {
        bind(&mut args, "--ro-bind", path);
    }
    for path in write {
        bind(&mut args, "--bind", path);
    }
    args.extend(["--ro-bind", "/dev/null", NO_BUS].map(String::from));
    let cwd = command.cwd.display().to_string();
    let bus = format!("unix:path={NO_BUS}");
    args.extend(
        ["--setenv", "DBUS_SESSION_BUS_ADDRESS", &bus, "--die-with-parent", "--chdir", &cwd, "--"].map(String::from),
    );
    args.push(command.program.display().to_string());
    args.extend(command.args);
    SpawnCommand { program: bwrap.to_path_buf(), args, ..command }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProcessSpawner;

    fn command() -> SpawnCommand {
        SpawnCommand {
            program: PathBuf::from("umu-run"),
            args: vec!["/games/PC/Game/game.exe".into()],
            env: [("WINEPREFIX".to_owned(), "/data/prefixes/Game".to_owned())].into(),
            cwd: PathBuf::from("/games/PC/Game"),
            log: None,
            keep_running: false,
        }
    }

    fn desktop() -> Desktop {
        Desktop {
            runtime_dir: Some("/run/user/1000".into()),
            sockets: vec!["/run/user/1000/wayland-0".into(), "/run/user/1000/pipewire-0".into()],
            files: vec!["/run/user/1000/xauth_abc".into()],
        }
    }

    fn wrapped() -> SpawnCommand {
        let write = [PathBuf::from("/games/PC/Game"), PathBuf::from("/data/prefixes/Game")];
        let read = [PathBuf::from("/data/umu")];
        wrap(command(), Path::new("/usr/bin/bwrap"), &read, &write, Some(Path::new("/home/player")), &desktop())
    }

    /// The `n` words after each `word` in `args`.
    fn after(args: &[String], word: &str, n: usize) -> Vec<Vec<String>> {
        args.windows(n + 1).filter(|w| w[0] == word).map(|w| w[1..].to_vec()).collect()
    }

    fn pair(path: &str) -> Vec<String> {
        vec![path.to_owned(), path.to_owned()]
    }

    #[test]
    fn the_game_s_command_follows_bwrap_s_words_as_it_was() {
        let run = wrapped();
        assert_eq!(run.program, Path::new("/usr/bin/bwrap"));
        let at = run.args.iter().position(|a| a == "--").unwrap();
        assert_eq!(run.args[at + 1..], ["umu-run", "/games/PC/Game/game.exe"]);
        assert_eq!(after(&run.args, "--chdir", 1), [vec!["/games/PC/Game".to_owned()]]);
        assert_eq!(run.cwd, Path::new("/games/PC/Game"));
        assert_eq!(run.env, command().env, "bwrap passes the game's environment on");
    }

    #[test]
    fn it_writes_only_the_folders_asked_for_and_reads_the_system() {
        let run = wrapped();
        assert_eq!(after(&run.args, "--bind", 2), [pair("/games/PC/Game"), pair("/data/prefixes/Game")]);
        // A folder at its own path; the one other read-only bind is the empty bus.
        let read_only = after(&run.args, "--ro-bind", 2);
        assert_eq!(read_only, [pair("/data/umu"), vec!["/dev/null".to_owned(), NO_BUS.to_owned()]]);
        let read = after(&run.args, "--ro-bind-try", 2);
        for path in ["/usr", "/etc", "/run/udev", "/tmp/.X11-unix"] {
            assert!(read.contains(&pair(path)), "{path}");
        }
        assert!(!read.iter().chain(&after(&run.args, "--bind", 2)).any(|p| p[0].starts_with("/home")), "{read:?}");
    }

    #[test]
    fn its_home_and_runtime_folder_are_empty_and_have_only_the_display_and_sound() {
        let run = wrapped();
        let dirs = after(&run.args, "--dir", 1);
        assert!(dirs.contains(&vec!["/home/player".to_owned()]), "{dirs:?}");
        assert!(dirs.contains(&vec!["/run/user/1000".to_owned()]), "{dirs:?}");
        let read = after(&run.args, "--ro-bind-try", 2);
        for socket in ["/run/user/1000/wayland-0", "/run/user/1000/pipewire-0", "/run/user/1000/xauth_abc"] {
            assert!(read.contains(&pair(socket)), "{socket}");
        }
        assert!(!read.contains(&pair("/run/user/1000/bus")), "no session D-Bus");
        // Set to no bus, not unset: without the variable, libdbus starts a new dbus-launch at
        // each try, and a game that keeps trying fills the process table.
        let address = format!("unix:path={NO_BUS}");
        assert_eq!(after(&run.args, "--setenv", 2), [vec!["DBUS_SESSION_BUS_ADDRESS".to_owned(), address]]);
        // The path is there, for Proton's own container, which binds the bus's socket into
        // itself: an empty device, which no program can talk to.
        assert!(run.args.windows(3).any(|w| w == ["--ro-bind", "/dev/null", NO_BUS]), "{:?}", run.args);
        assert!(!run.args.iter().any(|a| a == "--unsetenv"), "{:?}", run.args);
    }

    #[test]
    fn a_folder_asked_for_comes_after_the_empty_ones_so_it_shows_through() {
        let run = wrapped();
        let last_dir = run.args.iter().rposition(|a| a == "--dir").unwrap();
        let first_bind = run.args.iter().position(|a| a == "--bind").unwrap();
        assert!(last_dir < first_bind, "{:?}", run.args);
        let tmp = run.args.iter().position(|a| a == "/tmp").unwrap();
        let x11 = run.args.iter().position(|a| a == "/tmp/.X11-unix").unwrap();
        assert!(tmp < x11, "the X11 sockets in the empty /tmp");
    }

    /// The words above with the real bubblewrap: a shell in the sandbox says what it sees. Runs
    /// with NEXUS_SANDBOX_TESTS=1; it needs bwrap on the machine.
    #[test]
    fn bubblewrap_shows_the_game_its_folders_and_hides_the_rest() {
        if std::env::var_os("NEXUS_SANDBOX_TESTS").is_none() {
            eprintln!("SKIPPED: set NEXUS_SANDBOX_TESTS=1 to run a shell through bubblewrap");
            return;
        }
        let bwrap = crate::TokioSpawner.which("bwrap").expect("bwrap on the PATH");
        let root = tempfile::tempdir().unwrap();
        let (home, game, other) =
            (root.path().join("home"), root.path().join("home/Games/Game"), root.path().join("home/Secret"));
        for dir in [&game, &other] {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(other.join("key"), "secret").unwrap();
        // Another program's shared memory.
        let shared = PathBuf::from(format!("/dev/shm/nexus-sandbox-test-{}", std::process::id()));
        std::fs::write(&shared, "shared").unwrap();
        let script = "ls -A \"$HOME\" | tr '\\n' ' '; echo; ls -A \"$HOME/Games\"; echo saved > save.txt; \
                      cat \"$HOME/Secret/key\" 2>/dev/null || echo no-secret; echo \"bus=$DBUS_SESSION_BUS_ADDRESS\"; test -c /run/nexus-sandbox/no-session-bus && echo bus-path; echo \"shm=$(ls -A /dev/shm)\"";
        let command = SpawnCommand {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            env: [("HOME".to_owned(), home.display().to_string())].into(),
            cwd: game.clone(),
            log: None,
            keep_running: false,
        };
        let run = wrap(command, &bwrap, &[], std::slice::from_ref(&game), Some(&home), &Desktop::default());
        let out = std::process::Command::new(&run.program)
            .args(&run.args)
            .envs(&run.env)
            .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/0/bus")
            .output()
            .unwrap();
        std::fs::remove_file(&shared).unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let said = String::from_utf8_lossy(&out.stdout);
        let lines: Vec<&str> = said.lines().collect();
        assert_eq!(
            lines,
            ["Games ", "Game", "no-secret", &format!("bus=unix:path={NO_BUS}"), "bus-path", "shm="],
            "{said}"
        );
        assert_eq!(std::fs::read_to_string(game.join("save.txt")).unwrap(), "saved\n", "it writes its folder");
    }
}
