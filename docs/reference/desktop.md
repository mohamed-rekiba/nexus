# The desktop

This page is for contributors who work on windows, focus, controllers and start at login. It
says what Nexus does on Linux, macOS and Windows, and which tool or permission each system
needs.

Terms follow [GLOSSARY.md](../../GLOSSARY.md). For the layers around this code, read
[../architecture.md](../architecture.md). For the check commands, read
[../development.md](../development.md).

## Where the code is

The core never calls the system. It calls four ports in `crates/nexus-core/src/ports.rs`. The
app plugs in the real adapters in `crates/nexus-app/src/main.rs`.

| Port | Real adapter | Default when none is plugged in |
| --- | --- | --- |
| `Windows` | `SystemWindows` in `desktop.rs` | `NoWindows`: front is `Front::Unknown`, nothing comes to the front |
| `LoginSession` | `SystemLoginSession` in `desktop.rs` | `NoLoginSession`: refuses start at login and package installs |
| `Opener` | `SystemOpener` in `desktop.rs` | `NoOpener`: refuses links and paths |
| `ProcessTree` | `SystemProcesses` in `processes.rs` | `GroupTree` in `nexus-core/src/process.rs`: the process group of the root, and nothing on Windows |

The sessions subsystem (`nexus-core/src/sessions/`, the watch is `watch.rs`) uses `Windows`
and `ProcessTree` to follow a running game. `preferences.rs` uses `LoginSession`.

Two more pieces run on the UI thread and are not ports of the core:

- `platform.rs` is the `Platform` of the app shell. It raises the launcher's window with winit,
  moves it to a monitor, and tells the shell whether the window has the focus.
- `gamepad.rs` reads controllers and sends each press to the app shell.

## What each system needs

Nexus finds a window by the process ids of a game. Each system does it with its own tool.

| System | Tool | The player must have |
| --- | --- | --- |
| Linux, X11 | `xdotool` | `xdotool` on the `PATH`, and `DISPLAY` set |
| Linux, Gamescope | `xdotool` and `xprop` | Both on the `PATH`. Gamescope sets `GAMESCOPE_WAYLAND_DISPLAY` |
| Linux, KDE Plasma on Wayland | KWin scripting over D-Bus | A session bus and KWin. No program to install |
| Linux, another Wayland desktop | `xdotool`, for games under XWayland only | `DISPLAY` set (XWayland) |
| macOS | `osascript` driving System Events | The Accessibility permission, and permission to control System Events |
| Windows | Win32 calls in `windows-sys` | Nothing to install |

The checks run in this order on Linux, in `desktop.rs` (`system::front`, `system::show`,
`system::has_window`):

1. KWin, when `kwin::Kwin::get()` finds it.
2. Gamescope, when `GAMESCOPE_WAYLAND_DISPLAY` is set.
3. `xdotool`.

### How Nexus picks KWin

`kwin::Kwin::get()` connects once per run. It returns `None` unless all of these hold:

- `WAYLAND_DISPLAY` is set.
- `XDG_CURRENT_DESKTOP` has a `KDE` part.
- `GAMESCOPE_WAYLAND_DISPLAY` is not set.
- A session bus is there.
- KWin answers a first test script within `ANSWER_TIME`.

When it returns `None`, the `xdotool` path stands in.

### Tell the player what is missing

`Windows::problem` returns a sentence that names the fix. The sessions subsystem asks once per
run, when the first game starts, and shows the answer as a notice.

- Linux: `Install xdotool so games come to the front and the launcher comes back`, with
  `xdotool and xprop` under Gamescope. No notice when KWin works, when `DISPLAY` is not set, or
  when the tools are on the `PATH`.
- macOS: a sentence about the Accessibility permission and Automation. Nexus tests it by
  counting the windows of its own process through System Events. If that fails, it shows the
  notice.
- Windows: never.

## Bring a window to the front

`Windows::show_game(ids)` brings a window of one of the processes `ids` to the front. It
returns `false` when none has a window yet or the system refuses.

### Linux, X11

`xdotool search --onlyvisible --pid <id>` lists the windows of each process. Nexus runs
`xdotool windowactivate` on the last window found.

`front()` runs `xdotool getactivewindow getwindowpid`.

### Linux, Gamescope

Gamescope has no window manager. It shows the windows whose `STEAM_GAME` id comes first in the
root property `GAMESCOPECTRL_BASELAYER_APPID`.

- For a game, Nexus finds its windows with `xdotool`, sets `STEAM_GAME` on each with `xprop` to
  the id of the game's first process, and sets `GAMESCOPECTRL_BASELAYER_APPID` to that id, then
  `LAUNCHER_APPID` (7), so the launcher stays behind it.
- For the launcher, Nexus sets `LAUNCHER_APPID` on the launcher's window and puts it alone in
  the list.
- `front()` reads `GAMESCOPE_FOCUSED_WINDOW` from the root window with `xprop`, then asks
  `xdotool getwindowpid` for its process.

Gamescope's Steam mode shows no window that has no id. `desktop::claim_gamescope` sets the
launcher's id once its window exists. The app shell calls it through `Platform::claim_window`
on each tick until it returns `true`. Outside Gamescope, `claim_window` returns `true` at once.

### Linux, KDE Plasma on Wayland

`kwin::Kwin::ask` runs one question at a time:

1. Write a JavaScript file in the temp folder. `kwin::script` puts the prelude in front of the
   question.
2. Load it with `org.kde.kwin.Scripting.loadScript` on `org.kde.KWin`, and run it. The run path
   is `/Scripting/Script<id>` on KWin 6 and `/<id>` on KWin 5.
3. The script calls `answer` on the launcher's `/nexus` object (`org.nexus.Answer`) over
   D-Bus. Nexus waits `ANSWER_TIME` for it.
4. Unload the script and delete the file.

The prelude uses KWin 6 names (`windowList`, `activeWindow`) and falls back to KWin 5 names
(`clientList`, `activeClient`). A window counts when its pid is in `ids` and it is a normal
window or a dialog. `show` un-minimizes the last matching window, then makes it active.

KWin sees Wayland and XWayland windows alike.

### Linux, another Wayland desktop

No program may raise another program's window. Nexus raises its own window through winit
(`platform::raise_launcher`). A game under XWayland still comes to the front through
`xdotool`, because `DISPLAY` is set.

### macOS

Every call is an `osascript -e` command to System Events:

- `front()`: `unix id of first process whose frontmost is true`.
- `has_window(ids)`: `count windows of (first process whose unix id is <id>)` for each id.
- `show(ids)`: for the first process with a window, `set frontmost of ... to true`.

Without the Accessibility permission every call fails. `front()` is then `Front::Unknown`, and
the core falls back to the last window it showed.

macOS keeps a program running after its last window closes. For that reason
`SystemWindows::window_close_ends_game` returns `true` only on macOS.

### Windows

`windows()` calls `EnumWindows` and keeps the visible windows with no owner (the windows of
the taskbar) whose process is in `ids`. `front()` calls `GetForegroundWindow`.

Windows lets only the program in front give the front away. `bring_to_front` restores a
minimized window, joins the input of the window in front with `AttachThreadInput` for the
call, runs `BringWindowToTop` and `SetForegroundWindow`, and detaches again.

### The launcher's own window

`SystemWindows::show_launcher` calls the `raise` closure first. In the app, that is
`platform::raise_launcher`: on the UI thread it un-minimizes the window (not on Wayland, which
cannot tell and warns on every try), then calls `focus_window`. Then it calls the system's
`show` for the launcher's own process id. That call is what Gamescope needs. Elsewhere it is a
fallback, and a failure logs a debug line.

## Follow a game

A game is its processes. A session lasts until none of them is left. This is how
`SystemProcesses::members(root)` finds them:

- Unix (Linux and macOS): `pgrep -g <root>` lists the process group. The launcher starts each
  game in a group of its own (`process_group(0)` in `nexus-core/src/process.rs`), so the group
  holds the programs the game started. `root` is added when `kill -0` says it runs. Then
  `ps -A -o pid=,ppid=` gives the parents, and `and_below` adds every process below the ones
  found. A sandbox that starts its programs in a session of its own, as Proton's
  pressure-vessel does, stays covered.
- Windows: a ToolHelp snapshot gives the parents. There are no process groups, so the game is
  the root and every process below it. An empty list means the root is gone.

A start script that starts the game in the background and exits keeps its session, because
the background program is still in the group.

### A game another program starts

A launch can set `LaunchSpec::game_process`, as `steam steam://rungameid/...` does. The launcher
cannot follow the process it started, so it follows another one.

1. Before the launch, `ProcessTree::find(text)` lists the processes that already have those
   words in their command line.
2. After the launch, the watch calls `find` again each time and takes the first new match. A
   process counts only when its parent does not match too (`topmost`), so Steam's first
   process of a game is taken, not one it started.
3. The program that the launch started is only waited for in the background. It is never
   stopped, because it may have become Steam's client.
4. If nothing shows within `HostConfig::hand_over_timeout`, or Stop comes first, the session
   ends with nothing played.

On Unix, `find` reads `ps -A -ww -o pid=,ppid=,args=`. On Windows it returns nothing: the
ToolHelp snapshot has no command lines, so this kind of launch is not found there. See
Limitations.

### The watch loop

`sessions/watch.rs` runs the loop. Each look asks the desktop through `spawn_blocking`,
because the tools are programs.

| Stage | What happens |
| --- | --- |
| Until the game shows | Every `SHOW_EVERY` (or `HostConfig::watch_every` if shorter) the watch lists the processes and calls `show_game` once. |
| The game shows | `show_game` returned `true`, or `HostConfig::show_timeout` passed. The session is then `shown`. |
| Shown, but the desktop cannot say | After the timeout, the watch asks `front()`. If it is `Front::Unknown`, the game counts as in front, since its own window took the screen. |
| After it shows | Every `HostConfig::watch_every` the watch reads the processes, `front()` and `has_window`. |
| The end | The game ends when no process is left and the first one exited. |

After each look, the watch sets `game_in_front`:

- `Front::Launcher`: false.
- `Front::Other(id)`: true when `id` is one of the game's processes.
- `Front::Unknown`: no change. The last window Nexus showed counts.

On macOS, a game also ends when its window is gone for `WINDOW_GONE_FOR` looks in a row. The
watch counts only after it has seen a window of the game, so a desktop that cannot see the
window (a native Wayland window under `xdotool`) never ends a game this way.

When the first process of the game exits, the watch kills what is left in its group. On
Windows it also runs a PowerShell walk (`orphans_of` in `process.rs`) over the processes
created since the game started.

### Stop

Stop asks the game to close, and kills it if it does not.

1. `ProcessHandle::terminate` signals the group: `kill -TERM -- -<group>` on Unix,
   `taskkill /PID <id> /T` on Windows. `ProcessTree::end(ids, false)` sends `kill -TERM` to each
   process in the list, or `taskkill /PID` on Windows.
2. The watch polls every `STOP_EVERY` until `HostConfig::stop_grace` passes.
3. It then kills: `ProcessHandle::kill`, then `ProcessTree::end(left, true)` (`kill -KILL`, or
   `taskkill /F`).

When the game ends and the player turned on `return_on_exit`, the core calls
`show_launcher`.

## The controller

`gamepad.rs` reads controllers with `gilrs` on a thread named `gamepad`. DualSense, DualShock 4,
Xbox and most pads with a standard mapping work. If `gilrs` cannot start, Nexus logs
`no controller support` and goes on without a pad.

### Buttons

`gamepad::button` maps `gilrs` buttons to `PadButton`. `PadButton::action` in
`nexus-ui-host/src/input.rs` maps those to keys for the theme.

| Button | `PadButton` | What the theme or host gets |
| --- | --- | --- |
| D-pad, left stick | `Up`, `Down`, `Left`, `Right` | Arrow keys |
| South (A, Cross) | `South` | Return |
| East (B, Circle) | `East` | Escape |
| North (Y, Triangle) | `North` | `/` |
| West (X, Square) | `West` | Insert |
| Start | `Start` | Menu |
| Select | `Select` | Delete |
| Left shoulder | `LeftShoulder` | Backtab |
| Right shoulder | `RightShoulder` | Tab |
| Left trigger | `LeftTrigger` | Page up |
| Right trigger | `RightTrigger` | Page down |
| PS or Xbox button, tap | `Guide` | Go to Home, host action |
| PS or Xbox button, hold | `GuideHeld` | Stop the game, or open the power menu, host action |

Only the left stick moves. It counts as a press when pushed past `PUSH` and lets go below
`RELEASE`. A value between the two keeps the last direction.

### Repeat

A held button or stick repeats after `FIRST_REPEAT`, then every `NEXT_REPEAT`. Each repeat is
marked, so the host sends it to the theme as a repeated key press, which a theme can ignore. The
reader waits at most `LONGEST_WAIT` for an event before it checks the held buttons.

The PS or Xbox button never repeats. Nothing goes out when it goes down. A release before
`GUIDE_HOLD` sends the tap. Still down at `GUIDE_HOLD`, it sends the hold once.
`GuideHold` in `gamepad.rs` implements this.

A controller that comes or goes clears every held button. `gamepad::spawn` also reports the
name of the first connected pad, for the launch splash.

### When the pad reaches the theme

`ThemeHost::pad` in `nexus-ui-host/src/host.rs` drops every press except the two Guide actions
when a game's window is in front, or when the launcher's window does not have the focus. The
launcher's focus comes from `Platform::window_focused`, which asks winit; with no window to ask,
it is `true`.

The two Guide actions:

- Tap: go to Home. While a game runs, nothing happens.
- Hold: while a game runs, send Stop to every running session. With none running, open the
  power menu.

`SessionCmd::ToggleFocus` exists in the sessions subsystem and switches between the launcher
and the game. No pad button sends it. See Limitations.

### SIGCHLD on Unix

Each program the launcher runs ends with `SIGCHLD`. On Linux it broke the wait in `gilrs` with
an error in the log. `ignore_child_exits` blocks `SIGCHLD` on the pad thread and on the thread
`gilrs` starts from it. Tokio still gets the signal on its own threads.

## Full screen and monitors

`Platform::move_to_monitor` in `platform.rs` moves the window to the monitor named in the
`monitor` pref, or in the `--monitor` option. It turns full screen off for the move and on
again after, so the window fills the new monitor. The app shell calls it on each tick when the
wanted monitor changes.

On macOS, full screen is winit's `simple_fullscreen` (`platform::simple_fullscreen`). The
window fills the screen in the current Space. In a Space of its own, bringing the app to the
front after a game closed did not switch back, and the launcher stayed out of sight.

## Start at login

The Linux, macOS and Windows entries are written by `desktop::login::set`. `SystemLoginSession`
uses it only when `update::login_program` finds the installed launcher: the AppImage file, the
binary in a package, or the binary in `Nexus.app`. A build from `cargo run` has none, so
`can_start_at_login` is `false`, and setting the pref fails with `this build can't start when
you log in`.

| System | Entry | Notes |
| --- | --- | --- |
| Linux | `nexus.desktop` in `$XDG_CONFIG_HOME/autostart`, or `~/.config/autostart` | `XDG_CONFIG_HOME` counts only if it is an absolute path. `Exec` is quoted as the Desktop Entry specification says. |
| macOS | `io.github.mohamed-rekiba.nexus.plist` in `~/Library/LaunchAgents` | `desktop::login::AGENT` names it. `RunAtLoad` is true. XML characters in the path are escaped. |
| Windows | Value `Nexus` in `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` | Written with `reg.exe add`, removed with `reg.exe delete`. The path is quoted. |

Turning it off when no entry exists is not an error.

If the system refuses, the error text goes to the view as `start_at_login_error`
(`LoginSession::start_at_login` returns it) and the pref does not change.

### Install the launcher's own package

On Linux only, `SystemLoginSession::install_package` runs
`pkexec <dnf install -y | apt-get install -y | pacman -U --noconfirm> <file>`. Exit code 126 or
127 from `pkexec` means the player dismissed the prompt or typed a wrong password
(`PackageError::Cancelled`). Any other failure carries the last non-empty line of `stderr`. On
other systems the trait default refuses.

## Open links and folders

`SystemOpener` starts the program and does not wait. A thread reaps it.

| System | Link | Path |
| --- | --- | --- |
| Linux | `xdg-open` | `xdg-open` |
| macOS | `open` | `open` |
| Windows | `cmd /C start "" <link>` | `explorer` |

## The Nexus session

`packaging/linux/nexus-session` runs Nexus with no desktop behind it. It sets
`NEXUS_SESSION=1` and starts `gamescope` with Nexus in it, or `cage` when Gamescope cannot run.
It also sets `SDL_VIDEO_DRIVER`, `SDL_VIDEODRIVER`, `QT_QPA_PLATFORM` and `GDK_BACKEND` to X11,
so games and emulators run on Gamescope's X server, which the focus rules above need.

Nexus talks to the script through its log. The app shell prints `window shown` on standard
output once the window has a size (`Platform::announce`). The script starts again when that
line does not come within `NEXUS_WATCHDOG` seconds. Nexus writes `log out` before it ends when
the player chose Log out, and the script ends the session. The script reads its settings from
`/usr/share/nexus/session.conf` and `/etc/nexus/session.conf`.

`nexus --supports gamescope` is what the script runs to ask whether this build can run in
Gamescope (`args::supports`).

## Tests

Run the unit tests of the app with the usual check command from
[../development.md](../development.md). They do not open windows. They cover:

- `desktop.rs`: the KWin script text, the autostart entry, the launch agent, the package
  commands, the `pkexec` exit codes, and writing and removing an entry in a temp folder.
- `processes.rs`: `and_below`, `topmost`, the `ps` line parser, and (Unix) finding a real
  `sleep` by its command line and following a script's background program.
- `gamepad.rs`: repeat times, the Guide tap and hold, the stick.

Two kinds of test need a real desktop. Both are opt in.

> Warning: `NEXUS_DESKTOP_TESTS=1` opens real windows, changes which one is in front, and
> writes and removes this user's start-at-login entry. The test puts back the value it found.
> Run it on a desktop you can spare for a minute.

```sh
NEXUS_DESKTOP_TESTS=1 cargo test -p nexus-app --test desktop
```

The test is `crates/nexus-app/tests/desktop.rs`. It has its own `main`, because a window must
run on the main thread of a process. It starts itself again as a helper window (the variable
`NEXUS_DESKTOP_HELPER_HIDE_MS` makes it a helper). It runs three tests:

- `a_window_shown_comes_to_the_front`: two helper windows, shown in turn with `show_game`.
- `a_window_that_closed_is_gone_while_its_program_runs`: a helper hides its window; `has_window`
  turns `Some(false)` while the helper still runs.
- `start_at_login_writes_the_system_s_entry_and_takes_it_out`: on, off, and off again.

A test skips, and says why, when the desktop does not answer: for example macOS without the
Accessibility permission. Set `NEXUS_DESKTOP_REQUIRE=1` too, and a skip fails the run. CI sets
both under Xvfb.

> Warning: `NEXUS_KWIN_TESTS=1` asks the real KWin and activates a real window.

```sh
NEXUS_KWIN_TESTS=1 cargo test -p nexus-app kwin_says
```

It needs a KDE Plasma session on Wayland. The test is in `desktop.rs`
(`kwin_says_which_window_is_in_front_and_whose_windows_there_are`). Without the variable it
returns and passes.

## Limitations

- Nexus has no way to bring a native Wayland game to the front on a Wayland desktop other than
  KDE Plasma. No program may raise another's window there. `xdotool` sees XWayland windows only.
  On that desktop with no `DISPLAY`, `has_window` and `front` return nothing.
- macOS needs the Accessibility permission. Without it, `front()` is `Front::Unknown`, no game
  comes to the front, and the launcher does not come back by itself.
- Windows cannot find a game that another program started: `ProcessTree::find` returns nothing,
  because the process snapshot has no command lines.
- `Windows::problem` on Windows never warns. A refused `SetForegroundWindow` shows only as a
  `false` from `show_game`.
- I could read the macOS and Windows code, but I could not run it. This page does not claim
  more than the code says for those two systems.
- No pad button sends `SessionCmd::ToggleFocus`. A tap of the PS or Xbox button goes Home and
  does nothing while a game runs.
- The `gamepad` thread keeps only the first connected controller's name for the launch splash,
  though `gilrs` reads all of them.
- The desktop test does not cover `SystemProcesses`, `SystemOpener` or the Gamescope path.
