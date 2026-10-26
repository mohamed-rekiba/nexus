//! The desktop's windows, through the system's own tools, as the old launcher did it.
//!
//! - macOS: `osascript` drives System Events. macOS asks once for the Accessibility
//!   permission; without it every call fails, the launcher can't see which window is in front,
//!   and the core falls back to the last window it showed.
//! - Linux on X11: `xdotool`. Under Gamescope (a console-style OS), which has no window manager,
//!   `xprop` sets the window ids Gamescope shows.
//! - Linux on KDE Plasma's Wayland: KWin's scripting, over D-Bus (`kwin`): a short script
//!   finds the window by its process and answers back over D-Bus. It sees Wayland and
//!   XWayland windows alike.
//! - Linux on another Wayland desktop: no program may raise another's window. The launcher
//!   raises itself through winit, and a game under XWayland still works through `xdotool`.
//! - Windows: the Win32 API.
//!
//! A game is its processes, as `processes.rs` lists them: every call here takes their ids.
//!
//! Start at login writes the system's own entry for the installed launcher (`login`): a
//! `nexus.desktop` in `$XDG_CONFIG_HOME/autostart` on Linux, a launch agent in
//! `~/Library/LaunchAgents` on macOS, and a value in the `Run` key of the registry on Windows,
//! through `reg.exe`.

use std::process::{Command, Stdio};

use nexus_core::{Desktop, Front};
#[cfg(target_os = "linux")]
use nexus_core::{PackageError, PackageManager};

/// The system's desktop. The launcher's own window comes to the front through `raise`, on the
/// UI thread: winit asks the system directly, with no permission needed.
pub struct SystemDesktop {
    pub raise: Box<dyn Fn() + Send + Sync>,
    /// The program the system starts at login: the installed launcher. `None` for a build
    /// that isn't installed.
    pub login: Option<std::path::PathBuf>,
}

/// What a helper program printed, or `None` when it failed or isn't there.
#[cfg(unix)]
fn run(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

#[cfg(target_os = "macos")]
mod system {
    use super::*;

    fn osascript(script: &str) -> Option<String> {
        run("osascript", &["-e", script])
    }

    pub fn front() -> Option<u32> {
        osascript(r#"tell application "System Events" to get unix id of first process whose frontmost is true"#)?
            .parse()
            .ok()
    }

    /// Whether a process of `ids` has a window; `None` when System Events doesn't answer.
    pub fn has_window(ids: &[u32]) -> Option<bool> {
        let mut answered = false;
        for id in ids {
            let process = format!("(first process whose unix id is {id})");
            let Some(n) = osascript(&format!(r#"tell application "System Events" to count windows of {process}"#))
            else {
                continue;
            };
            answered = true;
            if n.parse::<u32>().is_ok_and(|n| n > 0) {
                return Some(true);
            }
        }
        answered.then_some(false)
    }

    /// Why games can't come to the front: System Events refuses without the Accessibility
    /// permission.
    pub fn problem() -> Option<String> {
        let own = std::process::id();
        let script =
            format!(r#"tell application "System Events" to count windows of (first process whose unix id is {own})"#);
        osascript(&script).is_none().then(|| {
            "Allow the app that starts the launcher (Terminal, VS Code or the launcher itself) in System \
             Settings > Privacy & Security > Accessibility, and to control System Events under \
             Automation, so games come to the front and the launcher comes back"
                .to_owned()
        })
    }

    /// Brings the first process of `ids` that has a window to the front.
    pub fn show(ids: &[u32]) -> bool {
        ids.iter().any(|id| {
            let process = format!("(first process whose unix id is {id})");
            let windows = osascript(&format!(r#"tell application "System Events" to count windows of {process}"#));
            windows.and_then(|n| n.parse::<u32>().ok()).is_some_and(|n| n > 0)
                && osascript(&format!(r#"tell application "System Events" to set frontmost of {process} to true"#))
                    .is_some()
        })
    }
}

#[cfg(target_os = "linux")]
mod system {
    use super::*;

    /// The launcher's own Gamescope id; games use their process id.
    const LAUNCHER_APPID: u32 = 7;

    /// Running inside Gamescope: it sets this for the programs it starts.
    fn gamescope() -> bool {
        std::env::var_os("GAMESCOPE_WAYLAND_DISPLAY").is_some()
    }

    fn xdotool(args: &[&str]) -> Option<String> {
        std::env::var_os("DISPLAY")?;
        run("xdotool", args)
    }

    fn windows(ids: &[u32]) -> Vec<String> {
        ids.iter()
            .filter_map(|id| xdotool(&["search", "--onlyvisible", "--pid", &id.to_string()]))
            .flat_map(|out| out.split_whitespace().map(str::to_owned).collect::<Vec<_>>())
            .collect()
    }

    pub fn front() -> Option<u32> {
        if let Some(kwin) = super::kwin::Kwin::get() {
            return kwin.front();
        }
        if gamescope() {
            let focused = run("xprop", &["-root", "GAMESCOPE_FOCUSED_WINDOW"])?;
            let window = focused.rsplit('=').next().map(str::trim).filter(|w| w.parse::<u64>().is_ok())?;
            return xdotool(&["getwindowpid", window])?.parse().ok();
        }
        xdotool(&["getactivewindow", "getwindowpid"])?.parse().ok()
    }

    /// Gamescope shows the windows whose `STEAM_GAME` id comes first in the root window's
    /// `GAMESCOPECTRL_BASELAYER_APPID`. The launcher stays behind a game as the fallback.
    fn gamescope_show(ids: &[u32], appid: u32) -> bool {
        let windows = windows(ids);
        for w in &windows {
            run("xprop", &["-id", w, "-f", "STEAM_GAME", "32c", "-set", "STEAM_GAME", &appid.to_string()]);
        }
        if windows.is_empty() {
            return false;
        }
        let order = if appid == LAUNCHER_APPID { appid.to_string() } else { format!("{appid},{LAUNCHER_APPID}") };
        run(
            "xprop",
            &["-root", "-f", "GAMESCOPECTRL_BASELAYER_APPID", "32c", "-set", "GAMESCOPECTRL_BASELAYER_APPID", &order],
        );
        true
    }

    /// Under Gamescope, tags the launcher's window so Gamescope shows it: in its Steam mode it
    /// shows no window without an id. False until the window is there, and outside Gamescope.
    pub fn claim_gamescope() -> bool {
        gamescope() && gamescope_show(&[std::process::id()], LAUNCHER_APPID)
    }

    pub fn has_window(ids: &[u32]) -> Option<bool> {
        if let Some(kwin) = super::kwin::Kwin::get() {
            return kwin.has_window(ids);
        }
        std::env::var_os("DISPLAY")?;
        Some(!windows(ids).is_empty())
    }

    fn on_path(name: &str) -> bool {
        let Some(path) = std::env::var_os("PATH") else { return false };
        std::env::split_paths(&path).any(|dir| dir.join(name).is_file())
    }

    /// Why games can't come to the front: an X11 session without the tools. Wayland without
    /// X has no way at all, which the README says; it gets no notice.
    pub fn problem() -> Option<String> {
        if super::kwin::Kwin::get().is_some() {
            return None;
        }
        std::env::var_os("DISPLAY")?;
        let missing: Vec<&str> =
            ["xdotool"].into_iter().chain(gamescope().then_some("xprop")).filter(|tool| !on_path(tool)).collect();
        (!missing.is_empty()).then(|| {
            format!("Install {} so games come to the front and the launcher comes back", missing.join(" and "))
        })
    }

    pub fn show(ids: &[u32]) -> bool {
        if let Some(kwin) = super::kwin::Kwin::get() {
            return kwin.show(ids);
        }
        let own = std::process::id();
        if gamescope() {
            let appid = if ids.contains(&own) { LAUNCHER_APPID } else { ids.first().copied().unwrap_or(own) };
            return gamescope_show(ids, appid);
        }
        match windows(ids).last() {
            Some(w) => xdotool(&["windowactivate", w]).is_some(),
            None => false,
        }
    }
}

/// KWin, KDE Plasma's window manager, on Wayland: each question is a script KWin loads, runs
/// and unloads. The script answers by calling the launcher's `/nexus` object over D-Bus.
#[cfg(target_os = "linux")]
mod kwin {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::mpsc::{Receiver, Sender, channel};
    use std::sync::{Arc, LazyLock, Mutex};
    use std::time::Duration;

    use zbus::blocking::Connection;

    /// How long KWin gets to answer.
    const ANSWER_TIME: Duration = Duration::from_millis(500);

    /// The windows, and the active one, in KWin 6's names and KWin 5's.
    const PRELUDE: &str = "const windows = workspace.windowList ? workspace.windowList() : workspace.clientList();
const active = workspace.windowList ? workspace.activeWindow : workspace.activeClient;
function activate(w) {
    if (w.minimized) { w.minimized = false; }
    if (workspace.windowList) { workspace.activeWindow = w; } else { workspace.activeClient = w; }
}
function mine(w, ids) { return w && ids.indexOf(w.pid) >= 0 && (w.normalWindow || w.dialog); }
";

    /// The script for `question`, answering to the bus name `me`.
    pub fn script(me: &str, question: &str) -> String {
        format!(
            "{PRELUDE}function answer(text) {{ callDBus(\"{me}\", \"/nexus\", \"org.nexus.Answer\", \"answer\", String(text)); }}\n{question}\n"
        )
    }

    pub fn ids(ids: &[u32]) -> String {
        format!("[{}]", ids.iter().map(u32::to_string).collect::<Vec<_>>().join(","))
    }

    struct Answer(Arc<Mutex<Option<Sender<String>>>>);

    #[zbus::interface(name = "org.nexus.Answer")]
    impl Answer {
        // The name the script calls: zbus would make it `Answer`.
        #[zbus(name = "answer")]
        fn answer(&self, text: String) {
            if let Some(tx) = self.0.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
                let _ = tx.send(text);
            }
        }
    }

    pub struct Kwin {
        bus: Connection,
        waiting: Arc<Mutex<Option<Sender<String>>>>,
        /// One question at a time: each waits for its own answer.
        asking: Mutex<()>,
    }

    impl Kwin {
        /// KWin, on a Plasma Wayland session that has it; `None` elsewhere.
        pub fn get() -> Option<&'static Kwin> {
            static KWIN: LazyLock<Option<Kwin>> = LazyLock::new(Kwin::connect);
            KWIN.as_ref()
        }

        fn connect() -> Option<Kwin> {
            let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
            let kde = std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.split(':').any(|d| d == "KDE"));
            if !wayland || !kde || std::env::var_os("GAMESCOPE_WAYLAND_DISPLAY").is_some() {
                return None;
            }
            let waiting = Arc::new(Mutex::new(None));
            let bus = zbus::blocking::connection::Builder::session()
                .and_then(|b| b.serve_at("/nexus", Answer(waiting.clone())))
                .and_then(|b| b.build())
                .inspect_err(|e| tracing::info!(error = %e, "no session bus: KWin can't show games"))
                .ok()?;
            let kwin = Kwin { bus, waiting, asking: Mutex::new(()) };
            // KWin there and answering: else the X11 tools stand in.
            kwin.ask("answer(1);")?;
            tracing::info!("games come to the front through KWin");
            Some(kwin)
        }

        /// Runs `question` in KWin and returns what it answered.
        fn ask(&self, question: &str) -> Option<String> {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let _one = self.asking.lock().unwrap_or_else(|p| p.into_inner());
            let me = self.bus.unique_name()?.to_string();
            let name = format!("nexus-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
            let file = std::env::temp_dir().join(format!("{name}.js"));
            std::fs::write(&file, script(&me, question)).ok()?;
            let (tx, rx): (Sender<String>, Receiver<String>) = channel();
            *self.waiting.lock().unwrap_or_else(|p| p.into_inner()) = Some(tx);
            let file_path = file.to_string_lossy().into_owned();
            let ran = self.load(&file_path, &name).and_then(|id| self.run(id));
            let answer = ran.ok().and_then(|_| rx.recv_timeout(ANSWER_TIME).ok());
            let _ = self.scripting("unloadScript", &(name.as_str(),));
            *self.waiting.lock().unwrap_or_else(|p| p.into_inner()) = None;
            let _ = std::fs::remove_file(&file);
            answer
        }

        /// A call to KWin's scripting object.
        fn scripting<B>(&self, method: &str, body: &B) -> zbus::Result<zbus::message::Message>
        where
            B: zbus::export::serde::Serialize + zbus::zvariant::DynamicType,
        {
            self.bus.call_method(Some("org.kde.KWin"), "/Scripting", Some("org.kde.kwin.Scripting"), method, body)
        }

        /// Loads the script in `file` as `name`; KWin gives its id.
        fn load(&self, file: &str, name: &str) -> zbus::Result<i32> {
            self.scripting("loadScript", &(file, name))?.body().deserialize()
        }

        /// Runs script `id`: its object is `/Scripting/Script<id>` on KWin 6, `/<id>` on KWin 5.
        fn run(&self, id: i32) -> zbus::Result<()> {
            let run = |path: String| {
                self.bus.call_method(Some("org.kde.KWin"), path.as_str(), Some("org.kde.kwin.Script"), "run", &())
            };
            run(format!("/Scripting/Script{id}")).or_else(|_| run(format!("/{id}"))).map(|_| ())
        }

        pub fn front(&self) -> Option<u32> {
            self.ask("answer(active ? active.pid : 0);")?.parse().ok().filter(|pid| *pid != 0)
        }

        pub fn has_window(&self, ids: &[u32]) -> Option<bool> {
            let question = format!("const ids = {}; answer(windows.some(w => mine(w, ids)) ? 1 : 0);", self::ids(ids));
            self.ask(&question).map(|a| a == "1")
        }

        /// Brings the last window of `ids` to the front. `false` when none has one.
        pub fn show(&self, ids: &[u32]) -> bool {
            let question = format!(
                "const ids = {}; const found = windows.filter(w => mine(w, ids)); \
                 if (found.length) {{ activate(found[found.length - 1]); answer(1); }} else {{ answer(0); }}",
                self::ids(ids)
            );
            self.ask(&question).as_deref() == Some("1")
        }
    }
}

#[cfg(windows)]
#[allow(unsafe_code)] // Win32 calls are foreign functions; each one is wrapped in a safe function here.
mod system {
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, EnumWindows, GW_OWNER, GetForegroundWindow, GetWindow, GetWindowThreadProcessId, IsIconic,
        IsWindowVisible, SW_RESTORE, SetForegroundWindow, ShowWindow,
    };

    fn owner(window: HWND) -> u32 {
        let mut id = 0;
        // SAFETY: `id` outlives the call; a bad window gives 0.
        unsafe { GetWindowThreadProcessId(window, &mut id) };
        id
    }

    /// The visible top-level windows of processes `ids`, as in the taskbar.
    fn windows(ids: &[u32]) -> Vec<HWND> {
        struct Search<'a> {
            ids: &'a [u32],
            found: Vec<HWND>,
        }
        unsafe extern "system" fn each(window: HWND, search: LPARAM) -> windows_sys::core::BOOL {
            // SAFETY: `search` is the `Search` that `windows` passed, alive for the whole call.
            let search = unsafe { &mut *(search as *mut Search) };
            // SAFETY: `window` comes from EnumWindows.
            let shown = unsafe { IsWindowVisible(window) != 0 && GetWindow(window, GW_OWNER).is_null() };
            if shown && search.ids.contains(&owner(window)) {
                search.found.push(window);
            }
            1
        }
        let mut search = Search { ids, found: Vec::new() };
        // SAFETY: `each` reads `search` only while EnumWindows runs.
        unsafe { EnumWindows(Some(each), &mut search as *mut Search as LPARAM) };
        search.found
    }

    pub fn front() -> Option<u32> {
        // SAFETY: no arguments; a null window means none is in front.
        let window = unsafe { GetForegroundWindow() };
        (!window.is_null()).then(|| owner(window)).filter(|id| *id != 0)
    }

    pub fn has_window(ids: &[u32]) -> Option<bool> {
        Some(!windows(ids).is_empty())
    }

    /// Windows lets only the program in front give the front away. For the call, this thread
    /// joins the input of the window in front, which lifts that rule.
    fn bring_to_front(window: HWND) -> bool {
        // SAFETY: plain Win32 calls. The window can be gone by now; Win32 then fails the call,
        // which is safe. The input joined for the call is detached again.
        unsafe {
            if IsIconic(window) != 0 {
                ShowWindow(window, SW_RESTORE);
            }
            let front = GetForegroundWindow();
            let front_thread = if front.is_null() { 0 } else { GetWindowThreadProcessId(front, std::ptr::null_mut()) };
            let own = GetCurrentThreadId();
            let attached = front_thread != 0 && front_thread != own && AttachThreadInput(own, front_thread, 1) != 0;
            BringWindowToTop(window);
            let done = SetForegroundWindow(window) != 0;
            if attached {
                AttachThreadInput(own, front_thread, 0);
            }
            done
        }
    }

    pub fn show(ids: &[u32]) -> bool {
        windows(ids).into_iter().next().is_some_and(bring_to_front)
    }

    pub fn problem() -> Option<String> {
        None
    }
}

/// Puts the launcher on screen under Gamescope, once its window is there: Gamescope's Steam
/// mode shows nothing until a window carries an id, and winit sets none. True when done; false
/// while the window isn't up yet, and always outside Gamescope (Linux only).
pub fn claim_gamescope() -> bool {
    #[cfg(target_os = "linux")]
    return system::claim_gamescope();
    #[cfg(not(target_os = "linux"))]
    false
}

impl Desktop for SystemDesktop {
    fn front(&self) -> Front {
        match system::front() {
            Some(id) if id == std::process::id() => Front::Launcher,
            Some(id) => Front::Other(id),
            None => Front::Unknown,
        }
    }

    fn show_game(&self, ids: &[u32]) -> bool {
        system::show(ids)
    }

    fn show_launcher(&self) {
        (self.raise)();
        // Gamescope shows windows by id, which winit doesn't set; elsewhere this is the fallback.
        if !system::show(&[std::process::id()]) {
            tracing::debug!("the desktop helper could not bring the launcher to the front");
        }
    }

    fn has_window(&self, ids: &[u32]) -> Option<bool> {
        system::has_window(ids)
    }

    /// On macOS a program keeps running after its last window closes: the game is over then.
    fn window_close_ends_game(&self) -> bool {
        cfg!(target_os = "macos")
    }

    fn problem(&self) -> Option<String> {
        system::problem()
    }

    fn can_start_at_login(&self) -> bool {
        self.login.is_some()
    }

    fn start_at_login(&self, on: bool) -> Result<(), String> {
        let exe = self.login.as_deref().ok_or("this build isn't installed")?;
        login::set(exe, on)
    }

    // Linux only: deb, rpm and Arch packages. Elsewhere the trait's default refuses.
    #[cfg(target_os = "linux")]
    fn install_package(&self, file: &std::path::Path, manager: PackageManager) -> Result<(), PackageError> {
        let output = Command::new("pkexec")
            .args(package_command(manager))
            .arg(file)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| PackageError::Failed(format!("couldn't ask for the password (pkexec): {e}")))?;
        match output.status.code() {
            Some(0) => Ok(()),
            // pkexec: the prompt was dismissed (126), or the password wasn't accepted (127).
            Some(126 | 127) => Err(PackageError::Cancelled),
            code => {
                // Its last line says why, for example a dependency nothing provides.
                let said = String::from_utf8_lossy(&output.stderr);
                let why = said.lines().map(str::trim).rfind(|l| !l.is_empty()).map(str::to_owned);
                Err(PackageError::Failed(why.unwrap_or_else(|| format!("the package manager ended with {code:?}"))))
            }
        }
    }

    fn open_link(&self, link: &str) -> Result<(), String> {
        let (program, args): (&str, Vec<&str>) = if cfg!(target_os = "macos") {
            ("open", vec![link])
        } else if cfg!(windows) {
            ("cmd", vec!["/C", "start", "", link])
        } else {
            ("xdg-open", vec![link])
        };
        // Not waited for: without a desktop's own opener, xdg-open runs the handler itself, and
        // `steam` may become Steam's client and keep running. Reaped in the background.
        let mut opener = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("couldn't open the link ({program}): {e}"))?;
        std::thread::spawn(move || opener.wait());
        Ok(())
    }
}

/// The package manager's command that installs a local package file, which follows it.
#[cfg(target_os = "linux")]
fn package_command(manager: PackageManager) -> &'static [&'static str] {
    match manager {
        PackageManager::Dnf => &["dnf", "install", "-y"],
        PackageManager::Apt => &["apt-get", "install", "-y"],
        PackageManager::Pacman => &["pacman", "-U", "--noconfirm"],
    }
}

/// The system's entry that starts the launcher at login.
pub mod login {
    use std::path::Path;

    /// The launch agent's name on macOS: the app's bundle id.
    #[cfg(any(target_os = "macos", test))]
    pub const AGENT: &str = "io.github.mohamed-rekiba.nexus";

    /// The `Run` key of the registry on Windows, and the launcher's value in it.
    #[cfg(windows)]
    const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
    #[cfg(windows)]
    const RUN_VALUE: &str = "Nexus";

    #[cfg(unix)]
    fn home() -> Result<std::path::PathBuf, String> {
        std::env::var_os("HOME").map(Into::into).ok_or_else(|| "HOME isn't set".to_owned())
    }

    /// The folder of the autostart entries: `$XDG_CONFIG_HOME/autostart`, or
    /// `~/.config/autostart`.
    #[cfg(target_os = "linux")]
    pub fn autostart_dir() -> Result<std::path::PathBuf, String> {
        let config = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from).filter(|p| p.is_absolute());
        Ok(match config {
            Some(config) => config,
            None => home()?.join(".config"),
        }
        .join("autostart"))
    }

    #[cfg(target_os = "macos")]
    pub fn agents_dir() -> Result<std::path::PathBuf, String> {
        Ok(home()?.join("Library/LaunchAgents"))
    }

    #[cfg(target_os = "linux")]
    pub fn set(exe: &Path, on: bool) -> Result<(), String> {
        write_or_remove(&autostart_dir()?.join("nexus.desktop"), on.then(|| autostart_entry(exe)))
    }

    #[cfg(target_os = "macos")]
    pub fn set(exe: &Path, on: bool) -> Result<(), String> {
        write_or_remove(&agents_dir()?.join(format!("{AGENT}.plist")), on.then(|| launch_agent(exe)))
    }

    /// `reg.exe add` puts the quoted path in the `Run` key; `reg.exe delete` takes it out, when
    /// it is there.
    #[cfg(windows)]
    pub fn set(exe: &Path, on: bool) -> Result<(), String> {
        let reg = |args: &[&str]| -> Result<(), String> {
            let out = Command::new("reg.exe")
                .args(args)
                .stdin(Stdio::null())
                .output()
                .map_err(|e| format!("reg.exe didn't run: {e}"))?;
            if out.status.success() {
                return Ok(());
            }
            Err(format!("reg.exe: {}", String::from_utf8_lossy(&out.stderr).trim()))
        };
        if on {
            let quoted = format!("\"{}\"", exe.display());
            reg(&["add", RUN_KEY, "/v", RUN_VALUE, "/t", "REG_SZ", "/d", &quoted, "/f"])
        } else if reg(&["query", RUN_KEY, "/v", RUN_VALUE]).is_ok() {
            reg(&["delete", RUN_KEY, "/v", RUN_VALUE, "/f"])
        } else {
            Ok(())
        }
    }

    /// Writes `file` with `contents`, or removes it for `None`. A file that is already gone
    /// is fine.
    #[cfg(any(unix, test))]
    pub fn write_or_remove(file: &Path, contents: Option<String>) -> Result<(), String> {
        let shown = file.display();
        match contents {
            Some(contents) => {
                if let Some(dir) = file.parent() {
                    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
                }
                std::fs::write(file, contents).map_err(|e| format!("{shown}: {e}"))
            }
            None => match std::fs::remove_file(file) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("{shown}: {e}")),
                _ => Ok(()),
            },
        }
    }

    /// A desktop entry that starts `exe`. `Exec` quotes the path as the Desktop Entry
    /// specification says: in double quotes, with `"`, `` ` ``, `$` and `\` escaped.
    #[cfg(any(target_os = "linux", test))]
    pub fn autostart_entry(exe: &Path) -> String {
        let mut quoted = String::from("\"");
        for c in exe.to_string_lossy().chars() {
            if matches!(c, '"' | '`' | '$' | '\\') {
                quoted.push('\\');
            }
            quoted.push(c);
        }
        quoted.push('"');
        // A `\` in a value is an escape too, so each one doubles again.
        let exec = quoted.replace('\\', "\\\\");
        format!(
            "[Desktop Entry]\nType=Application\nName=Nexus\nComment=Games from every emulator, in one place\n\
             Exec={exec}\nIcon=nexus\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
        )
    }

    /// A launch agent that starts `exe` once, when the player logs in.
    #[cfg(any(target_os = "macos", test))]
    pub fn launch_agent(exe: &Path) -> String {
        let program = exe.to_string_lossy().replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>{AGENT}</string>
    <key>ProgramArguments</key><array><string>{program}</string></array>
    <key>RunAtLoad</key><true/>
    <key>ProcessType</key><string>Interactive</string>
</dict>
</plist>
"#
        )
    }
}

#[cfg(test)]
mod tests {
    /// The script answers to the launcher's bus name, through `/nexus`, after KWin's names.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_kwin_question_answers_to_the_launcher() {
        let script = super::kwin::script(":1.42", "answer(1);");
        assert!(script.contains("callDBus(\":1.42\", \"/nexus\", \"org.nexus.Answer\", \"answer\""), "{script}");
        assert!(script.contains("workspace.windowList ? workspace.windowList() : workspace.clientList()"));
        assert!(script.ends_with("answer(1);\n"));
        assert_eq!(super::kwin::ids(&[7, 12]), "[7,12]");
    }

    /// Opt-in, `NEXUS_KWIN_TESTS=1`, on a Plasma Wayland session: asks the real KWin.
    #[cfg(target_os = "linux")]
    #[test]
    fn kwin_says_which_window_is_in_front_and_whose_windows_there_are() {
        if std::env::var_os("NEXUS_KWIN_TESTS").is_none() {
            return;
        }
        let kwin = super::kwin::Kwin::get().expect("KWin on this session");
        let front = kwin.front().expect("a window in front");
        assert_eq!(kwin.has_window(&[front]), Some(true));
        assert_eq!(kwin.has_window(&[1]), Some(false), "init has no window");
        assert!(kwin.show(&[front]), "the window in front comes to the front");
        assert!(!kwin.show(&[1]));
    }

    use std::path::Path;

    use super::login::*;

    #[test]
    fn the_autostart_entry_starts_the_launcher_with_its_path_quoted() {
        let entry = autostart_entry(Path::new("/home/sam/My Games/nexus"));
        assert!(entry.starts_with("[Desktop Entry]\nType=Application\n"), "{entry}");
        assert!(entry.contains("\nExec=\"/home/sam/My Games/nexus\"\n"), "{entry}");
        let odd = autostart_entry(Path::new("/opt/$a\"b/nexus"));
        assert!(odd.contains(r#"Exec="/opt/\\$a\\"b/nexus""#), "{odd}");
    }

    #[test]
    fn the_launch_agent_runs_the_launcher_at_load() {
        let agent = launch_agent(Path::new("/Applications/Nexus & Co.app/Contents/MacOS/nexus"));
        assert!(agent.contains(&format!("<key>Label</key><string>{AGENT}</string>")));
        assert!(agent.contains("<string>/Applications/Nexus &amp; Co.app/Contents/MacOS/nexus</string>"), "{agent}");
        assert!(agent.contains("<key>RunAtLoad</key><true/>"));
    }

    #[test]
    fn an_entry_is_written_and_removed_and_a_missing_one_is_fine() {
        let dir = std::env::temp_dir().join(format!("nexus-login-{}", std::process::id()));
        let file = dir.join("autostart/nexus.desktop");
        write_or_remove(&file, Some("entry".into())).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "entry");
        write_or_remove(&file, None).unwrap();
        assert!(!file.exists());
        write_or_remove(&file, None).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
