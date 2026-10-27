# The desktop: windows, the pad, full screen and start at login

This page describes how Nexus works with the desktop on macOS, Linux and Windows today:
which window is in front, how a game and the launcher come to the front, when the pad acts,
full screen and monitors, start at login, and how the app quits. The host and its ports are in
[architecture.md](../architecture.md). The screens are in [default-theme.md](default-theme.md).
The tools to install for each system are listed in the [README](../../README.md).

## The desktop's ports

The core reaches the desktop through three ports (`crates/nexus-core/src/ports.rs`). The app
plugs in an adapter for each from `crates/nexus-app/src/desktop.rs`. Tests plug in a fake for
each, and a stand-in is the default until the app plugs in a real one.

| Port | Does | App adapter | Test fake | Stand-in |
|---|---|---|---|---|
| `Windows` | Which window is in front, and bringing one to the front | `SystemWindows` | `FakeWindows` | `NoWindows` |
| `LoginSession` | Start at login, and installing the launcher's own package | `SystemLoginSession` | `FakeLoginSession` | `NoLoginSession` |
| `Opener` | Opening a link or a file with the system's opener | `SystemOpener` | `FakeOpener` | `NoOpener` |

| Call | Answer |
|---|---|
| `front()` | `Front::Launcher`, `Front::Other(pid)` or `Front::Unknown` |
| `show_game(ids)` | Brings a window of one of the game's processes to the front. `false` when none has one yet, or the system refused |
| `show_launcher()` | Brings the launcher's window to the front |
| `has_window(ids)` | Whether one of the game's processes has a window: `Some(true)`, `Some(false)`, or `None` when the system doesn't say |
| `window_close_ends_game()` | Whether a game whose windows closed is over while its processes run: true on macOS only |
| `problem()` | A missing tool or permission, with the fix, or `None` |
| `LoginSession::can_start_at_login()` | Whether this build can start at login |
| `LoginSession::start_at_login(on)` | Writes or removes the system's entry; the error is the system's reason |
| `LoginSession::install_package(file, manager)` | Installs the launcher's own deb, rpm or Arch package through `pkexec` (Linux only). A dismissed prompt or a wrong password (exit 126 or 127) is `Cancelled`; any other failure carries the package manager's last line |
| `Opener::open_link(link)` | Opens a link with `open`, `cmd /C start` or `xdg-open`, without waiting: a plugin's store link such as `steam://install/440`, or a page |
| `Opener::open_path(path)` | Opens a folder or a file with `open`, `explorer` or `xdg-open`, without waiting |

The other calls are `Windows`'.

The game's processes (`ids`) come from the `ProcessTree` port, which the app plugs in as
`SystemProcesses` (`crates/nexus-app/src/processes.rs`). On macOS and Linux they are the Unix
process group the game leads (`pgrep -g`) and every process below them (`ps -A -o pid=,ppid=`):
a sandbox such as Proton's pressure-vessel starts its programs in a session of their own, out of
the group. Windows has no process groups: there they are the game's process and every process
below it, from a process snapshot (`CreateToolhelp32Snapshot`).

The adapters only answer. The rules live once, in the sessions subsystem's watch
(`crates/nexus-core/src/sessions/watch.rs`).

## How each system answers

| System | Window in front | Bring a game up | Launcher up | Needs |
|---|---|---|---|---|
| macOS | `osascript`, System Events: the `unix id` of the frontmost process | Sets `frontmost` of the first group process that has a window | winit `focus_window`, then System Events | Accessibility permission, and Automation for System Events |
| Linux, X11 | `xdotool getactivewindow getwindowpid` | `xdotool windowactivate` on the last visible window of the group | winit, then `xdotool` | `xdotool` |
| Linux, Gamescope | `xprop -root GAMESCOPE_FOCUSED_WINDOW`, then `xdotool getwindowpid` | Sets `STEAM_GAME` on the game's windows and `GAMESCOPECTRL_BASELAYER_APPID` on the root window | The same, with the launcher's id `7` | `xdotool`, `xprop` |
| Linux, KDE Plasma on Wayland | A KWin script: the active window's `pid` | A KWin script activates the group's last normal window or dialog | winit, then a KWin script | Nothing |
| Linux, another Wayland desktop | `Unknown` without `DISPLAY` | Only games under XWayland, through `xdotool` | winit's activation request, which the compositor may refuse | `xdotool` for XWayland games |
| Windows | The process that owns `GetForegroundWindow` | Restores a minimized window, then `BringWindowToTop` and `SetForegroundWindow` | winit, then the same Win32 calls | Nothing |

### macOS

Every call runs `osascript -e` with a one-line System Events script. Without the Accessibility
permission every call fails. `front()` is then `Unknown`, and the core falls back to the last
window it showed.

### Linux on X11 and under Gamescope

The `xdotool` calls need `DISPLAY`. Gamescope is detected by `GAMESCOPE_WAYLAND_DISPLAY`, which
Gamescope sets for the programs it starts. Gamescope has no window manager: it shows the windows
whose `STEAM_GAME` id comes first in `GAMESCOPECTRL_BASELAYER_APPID`. A game gets its own process
id as its app id, and the order is `<game>,7`, so the launcher stays behind the game as the
fallback. Showing the launcher sets the order to `7` alone.

`nexus --supports gamescope` exits 0, so a console session can tell that this launcher sets the
ids Gamescope shows. Any other feature exits 1. The check runs before anything else.

### KDE Plasma on Wayland

KWin is used when `WAYLAND_DISPLAY` is set, `XDG_CURRENT_DESKTOP` names `KDE`, and
`GAMESCOPE_WAYLAND_DISPLAY` is not set. The launcher then:

1. connects to the session bus and serves the object `/nexus` with the interface
   `org.nexus.Answer` (method `answer`);
2. asks KWin a first question, `answer(1);`. When KWin doesn't answer, the X11 path stands in
   for the whole run.

Each question is a short JavaScript file in the temp folder, named `nexus-<pid>-<n>.js`. The
launcher loads it through `org.kde.kwin.Scripting.loadScript`, runs it (`/Scripting/Script<id>`
on KWin 6, `/<id>` on KWin 5), waits up to 500 ms for the script's `callDBus` answer, then
unloads the script and deletes the file. One question runs at a time.

The script works with KWin 6's names (`windowList`, `activeWindow`) and KWin 5's (`clientList`,
`activeClient`). A window counts when its `pid` is in the group and it is a normal window or a
dialog. Activating a minimized window un-minimizes it first. KWin sees Wayland and XWayland
windows alike, so this path needs no tool and shows no notice.

### Another Wayland desktop

No program may raise another program's window there. Without `DISPLAY`, `front()` is
`Unknown`, `has_window` is `None`, and a game can't be brought up. The launcher still raises
itself through winit. A game under XWayland comes up through `xdotool` when `DISPLAY` is set.

### Windows

The Win32 calls are in one module of `desktop.rs`, the only place with `unsafe` there, with
`#[allow(unsafe_code)]` and a reason. Windows refuses the front to a background program, so for
the call the launcher's thread joins the input of the window in front (`AttachThreadInput`) and
detaches again afterwards. A window counts when it is visible and has no owner, as in the
taskbar.

## Bringing the launcher up

`show_launcher()` first calls `raise_launcher` (`crates/nexus-app/src/platform.rs`) on the UI
thread. It un-minimizes the window only when winit reports it minimized (`Some(true)`), then
calls `focus_window`. Wayland can't report a minimize, and asking it to undo one only warns.
The system path (System Events, `xdotool`, Gamescope ids, KWin or Win32) runs after it as the
fallback. On Gamescope it is what actually shows the launcher, since winit doesn't set the ids.

## The running game

### Start

`launch` in `crates/nexus-core/src/launch.rs`:

- One game at a time. A second Play fails with "<title> is running: stop it first", or "a game
  is starting: wait for it". A game can't start while its emulator installs or updates.
- The game's output goes to `logs/<plugin>/<game>.log` in the data folder. The log of the run
  before is kept as `<game>.previous.log`.
- The Running screen is pushed. Its state is `starting` until the game's window shows, then
  `running`, and `stopping` after Stop.
- At the first game of a run, the core asks `problem()` once and shows the answer as an info
  notice (see [Notices](#notices)).

### The running game

The running-game module runs while the game runs. The game ends when none of its processes is
left, whatever its first process does.

1. Every 250 ms it tries `show_game`. It stops trying once a window came up, or after
   `show_timeout` (20 s, in `HostConfig`).
2. The session is then marked shown, and the Running screen leaves `starting`. `game_in_front`
   becomes true when the game came up. It also becomes true when the desktop can't say which
   window is in front (`Front::Unknown`): the game's window took the screen as it opened, so the
   pad stays with the game until the PS button.
3. After that, every `watch_every` (1 s, in `HostConfig`) it reads the game's processes, and
   asks `front()` and `has_window()`:
   - `Launcher` sets `game_in_front` to false.
   - `Other(pid)` sets it to whether that process is one of the game's.
   - `Unknown` leaves it as it was: the last window the launcher showed counts.
4. Where `window_close_ends_game()` (macOS), a game whose window was seen and is then gone for
   `WINDOW_GONE_FOR` (3) reads in a row is stopped. Three, so a window that is remade (full
   screen on or off) doesn't count. A desktop that never saw the window (`None`) never stops the
   game this way. Elsewhere a closed window stops nothing: a launcher's own menu or setup
   window closes before the game's opens.

`game_in_front` is app state. `Core::game_in_front()` reads it and never asks the desktop.

### A game another program starts

A launch whose `LaunchSpec.game_process` is set hands the game over: Steam's plugin runs
`steam steam://rungameid/<id>`, which asks Steam's client to start it, and exits (or, when Steam
wasn't running, becomes the client). Then:

- the program started here is never stopped, at Stop, at the end, or when Nexus quits
  (`SpawnCommand.keep_running`): `steam` becomes Steam's client when Steam wasn't running;
- before the launch starts anything, the core notes the processes that already have those words
  (the same game started from Steam itself): they aren't this launch's game;
- every 250 ms the module asks `ProcessTree::find` for the processes with those words in their
  command line (`SteamLaunch AppId=<id> `: Steam's `reaper`, which runs the game and ends with
  it), for up to `hand_over_timeout` (2 min, in `HostConfig`), and takes the first new one;
- once found, that process is the game's first: the steps above follow it and the processes
  below it, and the session ends when they are gone;
- found never, the session ends with an error notice, "<title> didn't start: its game didn't
  show up within 120 seconds", and no play time or result prompt;
- Stop before it is found ends the session with nothing played: no notice, play time or result
  prompt.

`find` reads `ps -A -ww -o pid=,ppid=,args=` and returns each match whose parent doesn't match.
Windows has no command lines in its process snapshot, so a handed-over game isn't found there.

### Stop and exit

Stop asks the game to close first, so the emulator saves its caches:

- on Unix, `kill -TERM` to the process group, and to each of the game's processes outside it
  (for a handed-over game, to its processes only: the program started here is left alone);
- on Windows, `taskkill /PID <pid> /T`, and `taskkill` for each of the game's processes.

After `stop_grace` (4 s) what is still there is killed: `kill -KILL`, or `taskkill /F`.

A start script that starts the game in the background and exits keeps the session until the
game ends. A program left running among the game's processes, such as a daemon, keeps the
session too, until the player holds the PS button or presses Stop.

When the game ends, by itself or by Stop, what it started and left in its group is killed the
same way. On Windows a PowerShell walk by parent id then stops the processes created
since the game started, since `taskkill /T` only follows a running process.

At the end:

- the play time is added (sessions under 3 s don't count);
- a game that exits with an error, without Stop, posts an error notice with its art: "<title>
  closed right after it started (code 1). Its log: <path>", or "closed with an error" after 10 s;
- the result prompt is set once for each emulator build;
- with "Back to the launcher" on (`app.return_on_exit`, on by default), `show_launcher()` runs.

### The PS button

The Guide button (PS or Xbox) is not sent to the theme as a key. In
`crates/nexus-ui-host/src/host.rs`:

- A tap while a game runs does nothing: the game keeps the screen.
- A tap with no game running opens Home.
- A hold (600 ms) while a game runs sends `SessionCmd::Stop` for each running game, at once,
  with no question.
- A hold with no game running opens the power menu (`Vm.power-menu`).

The theme's Resume button calls `Host.show-game`, which shows the game again when the launcher
is in front of it.

## The pad acts only while the launcher has the focus

Controllers are read with `gilrs` on a thread of their own (`crates/nexus-app/src/gamepad.rs`),
whatever window is in front. The d-pad and the left stick move. A held button or stick repeats
after 380 ms, then every 85 ms. The stick counts as pushed past 0.6 and let go under 0.3. Each
press goes to the UI thread, which first sets the window's focus from winit's `has_focus()`.

A press reaches the theme only when no game window is in front (`game_in_front` is false) and
the launcher's window has the keyboard focus. Otherwise only the Guide button gets through.
This covers Wayland too: there no program can name the window in front, but each window still
learns when it gains or loses the focus.

The first controller's name goes to the launch splash ("Controller: …"). The gamepad thread
blocks `SIGCHLD`, so the end of each program the launcher runs doesn't break `gilrs`'s wait.

## Full screen and monitors

Full screen is the setting `app.fullscreen`, on by default. `--windowed` keeps a normal window
for one run. The UI host applies it at every render, and only changes it when it differs.

- macOS: simple full screen (`set_simple_fullscreen`). The window covers the menu bar and the
  Dock in the current Space. In a Space of its own, the app coming back after a game didn't
  switch back to that Space.
- Linux and Windows: Slint's full screen; on a monitor move, winit's borderless full screen on
  that monitor.

The app shell's tick (`AppShell::on_tick`, `crates/nexus-ui-host/src/shell.rs`) places the window
every 250 ms, through the shell's `Platform` (`crates/nexus-app/src/platform.rs` asks winit):

- It lists the monitors' names for the Settings row "Screen", which shows with two monitors or
  more.
- The wanted monitor is `--monitor <name>` for this run, else the saved `app.monitor`.
  `--monitor` is not saved. It wins until the player picks a screen in Settings.
- The window moves once per choice: full screen goes off, the window moves to the monitor's
  position, and full screen goes on again there. A window already on that monitor stays. The
  player may then drag the window elsewhere.

Monitors are matched by name, so two monitors of the same model look alike.

`NEXUS_SCALE=<n>` sets the size of everything for one run, from 0.8 to 1.4, over the "Size of
everything" setting.

## Start at login

The Settings row "Start at login" shows only when `can_start_at_login()` is true. That needs an
installed build: an AppImage (`APPIMAGE`), a binary inside `Nexus.app`, or a package folder with
`LICENSE` and `extensions/` beside the binary. A `cargo run` build can't, since the program
would move.

| System | Entry | Runs |
|---|---|---|
| Linux | `$XDG_CONFIG_HOME/autostart/nexus.desktop`, or `~/.config/autostart/nexus.desktop` | The AppImage file, or the package's binary; `Exec` quoted as the Desktop Entry specification says |
| macOS | `~/Library/LaunchAgents/io.github.mohamed-rekiba.nexus.plist`, with `RunAtLoad` | The binary inside `Nexus.app` |
| Windows | The `Nexus` value in `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, through `reg.exe add` and `reg.exe delete` | The binary, in double quotes |

Off removes the entry. An entry that is gone already is fine. The core saves `app.start_at_login`
only when the call succeeds. Otherwise it keeps the reason, and the row's hint shows it in red.

The Linux install script (`scripts/install.sh`, shipped as `install.sh` in the tarball) is
separate from this. `--uninstall` removes its start-at-login entry too.

## Notices

Each of these shows once a run, at the first game:

| Desktop | Notice |
|---|---|
| X11 without `xdotool` | "Install xdotool so games come to the front and the launcher comes back" |
| Gamescope without `xdotool` or `xprop` | "Install xdotool and xprop so …" (the missing ones) |
| macOS without the permission | "Allow the app that starts the launcher (Terminal, VS Code or the launcher itself) in System Settings > Privacy & Security > Accessibility, and to control System Events under Automation, so games come to the front and the launcher comes back" |

The X11 check runs whenever `DISPLAY` is set and KWin isn't in use. KDE Plasma, another Wayland
desktop without `DISPLAY`, and Windows get no notice.

## This PC

The "THIS PC" section in Settings, and the Game Hub's "ON THIS PC" card, come from the
`Hardware` port (`crates/nexus-app/src/hardware.rs`). The core reads it once a run, off the UI
thread.

- Processor and memory: `/proc/cpuinfo` and `/proc/meminfo` on Linux; `sysctl` on macOS;
  PowerShell's `Get-CimInstance` on Windows. AVX2 comes from the processor itself on x86-64.
- Graphics cards: the launcher runs itself again as `nexus --probe-gpu` and waits at most 10 s.
  The child loads the Vulkan loader (`libvulkan.so.1`, `vulkan-1.dll`, or on macOS
  `libvulkan.dylib`, then the MoltenVK the installed emulator builds carry), lists the devices,
  and prints one line per card. A broken driver can only take the child down. Without Vulkan,
  macOS falls back to `system_profiler` names, with the Vulkan version unknown.

Each emulator's manifest says what its games need (`[requires]`). The verdicts are "Should run
well", "May run slowly" (with the main reason, such as "Integrated GPU"), "Won't start" (such
as "No Vulkan 1.3") and "Can't tell yet". When an emulator's games won't start, an error notice
says so once for each PC, for example "This PC can't run PS5 games: No Vulkan 1.3". Its link
opens Settings. The config keeps a summary of the PC the notice was for (`app.pc_noticed`).

## Downloads through a VPN

"Download only through a VPN" (`app.vpn_only`) differs by system (`crates/nexus-app/src/vpn.rs`):

- Linux: a VPN is a WireGuard or OpenVPN interface in `/sys/class/net`, or another layer-3
  tunnel, up and with an address. Tailscale, ZeroTier, Docker, libvirt, LXC and Waydroid
  interfaces don't count. NetworkManager (`nmcli`) adds a VPN from a `.conf` or `.ovpn` file,
  and connects and disconnects the VPNs it knows.
- macOS: the system keeps `utun` interfaces up for its own services, so a VPN can't be told
  apart. Every interface that is up and has an address is listed, and the player picks the
  one the VPN app made.
- Windows: "Not available on Windows". librqbit 9.0.1 can't bind its connections to an
  interface there.

The core's rules (the kill switch, waiting downloads) are in
[archives-and-pc-games.md](archives-and-pc-games.md#downloads-through-a-vpn).

## Quitting

The app quits from Quit ("Quit Nexus", Ctrl+Q), Restart, Ctrl-C, `SIGTERM` or `SIGHUP`. A
signal quits Slint's event loop as the Quit button does. A second signal exits at once with
code 130. On Windows only Ctrl-C is caught.

After the event loop ends, `Core::shutdown` runs with a limit of 10 s:

1. the torrents stop first, so each writes down the pieces it has (see
   [archives-and-pc-games.md](archives-and-pc-games.md#torrents));
2. every plugin is deactivated, dependents first.

A shutdown that takes longer is logged, and the app quits anyway. Restart then starts the
launcher again with the same arguments.

## Tests

- The core's running-game rules are tested once, through `FakeWindows`.
- The pure parts of `desktop.rs` have unit tests: the autostart entry, the launch agent, the
  package manager's command and what `pkexec`'s exit means.
- The monitor placement and the session's lines (`window shown`, `log out`) are tested at the
  app shell, on `FakePlatform` (`tests/tests/seams/shell.rs`).
- `crates/nexus-app/tests/desktop.rs` tests `SystemWindows` and `SystemLoginSession` against the
  real windowing system.
  It opens helper windows (the test binary itself, run with `NEXUS_DESKTOP_HELPER_HIDE_MS`),
  brings them to the front and hides one. It also turns start at login on and off for this
  user, then puts back what was there. It runs only with `NEXUS_DESKTOP_TESTS=1`. With
  `NEXUS_DESKTOP_REQUIRE=1` too, a skip fails the run. A skip prints why.
- The KWin path has an opt-in test, `NEXUS_KWIN_TESTS=1`, on a Plasma Wayland session.

## Limits

- On a Wayland desktop other than KDE Plasma, only games under XWayland come to the front.
- On macOS without the Accessibility permission, the launcher can't tell which window is in
  front. The pad then stays with the game until the PS button.
- A game the launcher didn't start is not watched.
- Downloads only through a VPN is not available on Windows.
