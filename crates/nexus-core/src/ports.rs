//! What the core needs from the outside world. The app plugs in real ones; tests plug in fakes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use futures::{FutureExt, StreamExt as _};
use nexus_api::{HttpRequest, HttpResponse, PluginResult};
use serde::{Deserialize, Serialize};

/// The time for the core: what time it is, and waits. Every timer in the core goes through
/// it, so a test can move time instead of waiting for it.
pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;

    /// Ends `wait` from now.
    fn sleep(&self, wait: Duration) -> BoxFuture<'static, ()>;

    /// Ticks every `every`, the first one `every` from now. A tick that came late doesn't
    /// bring the next ones closer.
    fn interval(&self, every: Duration) -> BoxStream<'static, ()>;
}

impl dyn Clock + '_ {
    /// Ends at `deadline`, or at once when it has passed.
    pub fn sleep_until(&self, deadline: Instant) -> BoxFuture<'static, ()> {
        self.sleep(deadline.saturating_duration_since(self.now()))
    }

    /// `work`'s result, or `None` when `limit` passed first. `work` is dropped then.
    pub async fn timeout<T>(&self, limit: Duration, work: impl Future<Output = T>) -> Option<T> {
        tokio::select! {
            done = work => Some(done),
            () = self.sleep(limit) => None,
        }
    }
}

/// Real time, from tokio's timers.
pub struct TokioClock;

impl Clock for TokioClock {
    fn now(&self) -> Instant {
        // Tokio's, not the system's: a runtime whose time a test paused gives the paused time.
        tokio::time::Instant::now().into_std()
    }

    fn sleep(&self, wait: Duration) -> BoxFuture<'static, ()> {
        tokio::time::sleep(wait).boxed()
    }

    fn interval(&self, every: Duration) -> BoxStream<'static, ()> {
        let mut ticks = tokio::time::interval_at(tokio::time::Instant::now() + every, every);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        futures::stream::unfold(ticks, |mut ticks| async move {
            ticks.tick().await;
            Some(((), ticks))
        })
        .boxed()
    }
}

/// The body of a download, from some offset.
pub struct Body {
    /// `true` when the server started at the requested offset. `false` means it sent the whole
    /// file from byte 0.
    pub resumed: bool,
    /// The length of the whole file, if known.
    pub total: Option<u64>,
    pub chunks: BoxStream<'static, PluginResult<Vec<u8>>>,
}

/// What a download request got: the body, or a redirect to follow.
pub enum Opened {
    Body(Body),
    /// The `Location` of a 3xx answer, as the server sent it (it may be relative).
    Redirect(String),
}

/// Raw HTTP. It never follows redirects: the core checks every hop against the plugin's
/// allowlist and follows them itself. Permission checks happen before a request gets here.
#[async_trait]
pub trait HttpTransport: Send + Sync {
    /// A 3xx comes back as it is, with its `Location` header.
    async fn send(&self, request: HttpRequest) -> PluginResult<HttpResponse>;

    /// Starts a GET at byte `offset` (a Range request when `offset > 0`).
    async fn open(&self, url: &str, offset: u64) -> PluginResult<Opened>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub cwd: PathBuf,
    /// Where the program's output goes. `None` throws it away.
    pub log: Option<PathBuf>,
    /// It may outlive Nexus: a program that hands a game to another one can become that
    /// program (`steam` becomes Steam's client when Steam wasn't running), which Nexus never
    /// stops, not even when Nexus quits.
    pub keep_running: bool,
}

impl SpawnCommand {
    /// The command as one line for a POSIX shell: the folder, the environment values the
    /// launcher sets, the program and its arguments. A player can paste it to run the game by hand.
    pub fn shell_line(&self) -> String {
        let mut words = vec!["cd".to_owned(), quote(&self.cwd.to_string_lossy()), "&&".to_owned()];
        words.extend(self.env.iter().map(|(key, value)| format!("{key}={}", quote(value))));
        words.push(quote(&self.program.to_string_lossy()));
        words.extend(self.args.iter().map(|arg| quote(arg)));
        words.join(" ")
    }
}

/// A word as a POSIX shell reads it back: as it is when it holds only safe characters, else in
/// single quotes.
fn quote(word: &str) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "-_./=:,+@%".contains(c);
    if !word.is_empty() && word.chars().all(safe) {
        word.to_owned()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessExit {
    pub code: Option<i32>,
    pub success: bool,
}

/// A started program. On Unix it leads a process group of its own, so the programs it starts
/// stop with it.
#[async_trait]
pub trait ProcessHandle: Send {
    /// The system's id for the process, while it runs.
    fn id(&self) -> Option<u32>;
    async fn wait(&mut self) -> PluginResult<ProcessExit>;
    /// Asks the program to close, as the system does at log out, so it can save first.
    async fn terminate(&mut self) -> PluginResult<()>;
    /// Stops the program, and the programs it started, at once. It also works after the program
    /// ended, for what it left running.
    async fn kill(&mut self) -> PluginResult<()>;
}

#[async_trait]
pub trait ProcessSpawner: Send + Sync {
    async fn spawn(&self, command: SpawnCommand) -> PluginResult<Box<dyn ProcessHandle>>;

    /// Runs a program to its end with no input, and returns its standard output, at most
    /// `limit` bytes. `None` when it was still running after `timeout`; it is stopped then.
    async fn output(&self, command: SpawnCommand, timeout: Duration, limit: usize) -> PluginResult<Option<Vec<u8>>>;

    /// A program's real path, found by file name on the system's `PATH`.
    fn which(&self, name: &str) -> Option<PathBuf>;

    /// Runs a program to its end with no input, handing each line of its output and errors to
    /// `line`, and stops it when `cancelled` says so. Returns its exit code (-1 when a signal
    /// ended it). By default it runs through [`ProcessSpawner::output`] and hands the lines
    /// over at the end, which is enough for fakes.
    async fn stream(
        &self,
        command: SpawnCommand,
        line: &(dyn for<'a> Fn(&'a str) + Send + Sync),
        _cancelled: &(dyn Fn() -> bool + Send + Sync),
    ) -> PluginResult<i32> {
        let out = self.output(command, Duration::from_secs(3600), usize::MAX).await?.unwrap_or_default();
        String::from_utf8_lossy(&out).lines().for_each(line);
        Ok(0)
    }
}

/// Spawns real processes with `tokio::process`.
pub struct TokioSpawner;

/// The child, its id kept after the child is reaped (its process group can outlive it), and
/// when it started.
#[cfg_attr(not(windows), allow(dead_code))] // Only Windows' orphan sweep reads the start time.
struct TokioChild(tokio::process::Child, Option<u32>, std::time::SystemTime);

#[async_trait]
impl ProcessSpawner for TokioSpawner {
    async fn spawn(&self, command: SpawnCommand) -> PluginResult<Box<dyn ProcessHandle>> {
        let failed =
            |e: std::io::Error| nexus_api::PluginError::Other(format!("could not start {:?}: {e}", command.program));
        let mut process = tokio::process::Command::new(&command.program);
        process.args(&command.args).envs(&command.env).current_dir(&command.cwd).kill_on_drop(!command.keep_running);
        process.stdin(std::process::Stdio::null());
        if let Some(log) = &command.log {
            if let Some(dir) = log.parent() {
                std::fs::create_dir_all(dir).map_err(failed)?;
            }
            let out = std::fs::File::create(log).map_err(failed)?;
            process.stderr(out.try_clone().map_err(failed)?).stdout(out);
        } else {
            process.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        }
        #[cfg(unix)]
        process.process_group(0);
        let child = process.spawn().map_err(failed)?;
        let id = child.id();
        Ok(Box::new(TokioChild(child, id, std::time::SystemTime::now())))
    }

    async fn output(&self, command: SpawnCommand, timeout: Duration, limit: usize) -> PluginResult<Option<Vec<u8>>> {
        use tokio::io::AsyncReadExt;
        let mut child = tokio::process::Command::new(&command.program)
            .args(&command.args)
            .envs(&command.env)
            .current_dir(&command.cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| nexus_api::PluginError::Other(format!("could not start {:?}: {e}", command.program)))?;
        let stdout = child.stdout.take().expect("stdout is piped");
        let run = async {
            let mut out = Vec::new();
            stdout.take(limit as u64).read_to_end(&mut out).await?;
            // The read dropped the pipe: a program with more to say stops on it, not on us.
            child.wait().await?;
            Ok::<_, std::io::Error>(out)
        };
        // A real program runs in real time, whatever clock the core has.
        let clock: &dyn Clock = &TokioClock;
        match clock.timeout(timeout, run).await {
            Some(Ok(out)) => Ok(Some(out)),
            Some(Err(e)) => Err(nexus_api::PluginError::Other(format!("{:?}: {e}", command.program))),
            None => {
                let _ = child.kill().await;
                Ok(None)
            }
        }
    }

    async fn stream(
        &self,
        command: SpawnCommand,
        line: &(dyn for<'a> Fn(&'a str) + Send + Sync),
        cancelled: &(dyn Fn() -> bool + Send + Sync),
    ) -> PluginResult<i32> {
        use futures::StreamExt;
        use tokio::io::{AsyncBufReadExt, BufReader};
        let failed = |e: std::io::Error| nexus_api::PluginError::Other(format!("{:?}: {e}", command.program));
        let mut child = tokio::process::Command::new(&command.program)
            .args(&command.args)
            .envs(&command.env)
            .current_dir(&command.cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| nexus_api::PluginError::Other(format!("could not start {:?}: {e}", command.program)))?;
        let mut out = BufReader::new(child.stdout.take().expect("stdout is piped")).lines();
        let mut err = BufReader::new(child.stderr.take().expect("stderr is piped")).lines();
        let (mut out_open, mut err_open) = (true, true);
        let mut tick = TokioClock.interval(Duration::from_millis(250));
        while out_open || err_open {
            tokio::select! {
                read = out.next_line(), if out_open => match read.map_err(failed)? {
                    Some(text) => line(&text),
                    None => out_open = false,
                },
                read = err.next_line(), if err_open => match read.map_err(failed)? {
                    Some(text) => line(&text),
                    None => err_open = false,
                },
                _ = tick.next() => if cancelled() {
                    let _ = child.kill().await;
                    break;
                },
            }
        }
        Ok(child.wait().await.map_err(failed)?.code().unwrap_or(-1))
    }

    fn which(&self, name: &str) -> Option<PathBuf> {
        let names: Vec<String> = if cfg!(windows) && Path::new(name).extension().is_none() {
            vec![format!("{name}.exe"), name.into()]
        } else {
            vec![name.into()]
        };
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path).flat_map(|dir| names.iter().map(move |n| dir.join(n))).find(|p| p.is_file())
    }
}

/// Sends `signal` to the process group `group` with the system's `kill`: this crate has no
/// unsafe code to call it directly.
#[cfg(unix)]
async fn signal_group(group: u32, signal: &str) -> PluginResult<()> {
    // A failed exit status means the group is already gone, which is what we wanted.
    tokio::process::Command::new("kill")
        .args([&format!("-{signal}"), "--", &format!("-{group}")])
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map_err(|e| nexus_api::PluginError::Other(format!("kill: {e}")))?;
    Ok(())
}

/// Runs `taskkill` on process `id` and the processes it started. Without `force`, it asks
/// their windows to close.
#[cfg(windows)]
async fn taskkill(id: u32, force: bool) -> PluginResult<()> {
    let mut args = vec!["/PID".to_owned(), id.to_string(), "/T".to_owned()];
    if force {
        args.push("/F".into());
    }
    // A failed exit status means the processes are already gone, which is what we wanted.
    tokio::process::Command::new("taskkill")
        .args(&args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map_err(|e| nexus_api::PluginError::Other(format!("taskkill: {e}")))?;
    Ok(())
}

/// Kills what process `id` started, and what those started, after `id` ended: `taskkill /T`
/// follows the tree from a running process only. A child keeps its parent's id, so a walk by
/// parent id still finds it. Windows can give a dead process's id to a new one, so the walk
/// takes only processes created since the game `started`: a stranger with a reused parent id
/// is older than that, or not below the game.
#[cfg(windows)]
async fn orphans_of(id: u32, started: std::time::SystemTime) {
    let since = started.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let script = format!(
        "$since = [DateTimeOffset]::FromUnixTimeSeconds({since}).LocalDateTime; \
         $all = Get-CimInstance Win32_Process; $parents = @({id}); $seen = @(); \
         while ($parents.Count -gt 0) {{ \
           $next = @($all | Where-Object {{ $parents -contains $_.ParentProcessId -and $seen -notcontains $_.ProcessId -and $_.CreationDate -ge $since }}); \
           $seen += $next.ProcessId; $parents = @($next.ProcessId) }}; \
         $seen | ForEach-Object {{ Stop-Process -Id $_ -Force -ErrorAction SilentlyContinue }}"
    );
    let ran = tokio::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await;
    if let Err(e) = ran {
        tracing::debug!(error = %e, "could not look for what the game left running");
    }
}

#[async_trait]
impl ProcessHandle for TokioChild {
    fn id(&self) -> Option<u32> {
        self.0.id()
    }

    async fn wait(&mut self) -> PluginResult<ProcessExit> {
        let status = self.0.wait().await.map_err(|e| nexus_api::PluginError::Other(e.to_string()))?;
        Ok(ProcessExit { code: status.code(), success: status.success() })
    }

    async fn terminate(&mut self) -> PluginResult<()> {
        let Some(id) = self.1 else { return Ok(()) };
        #[cfg(unix)]
        return signal_group(id, "TERM").await;
        #[cfg(windows)]
        return taskkill(id, false).await;
    }

    async fn kill(&mut self) -> PluginResult<()> {
        // By the kept id: after the first process ends, the rest of its group can still run.
        #[cfg(unix)]
        if let Some(id) = self.1 {
            signal_group(id, "KILL").await?;
        }
        #[cfg(windows)]
        if let Some(id) = self.1 {
            taskkill(id, true).await?;
            orphans_of(id, self.2).await;
        }
        match self.0.kill().await {
            // Already gone: the group signal reached it first.
            Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => Ok(()),
            other => other.map_err(|e| nexus_api::PluginError::Other(e.to_string())),
        }
    }
}

/// Which program's window the player sees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Front {
    Launcher,
    /// Another program's window, by process id.
    Other(u32),
    /// The system doesn't say, or the launcher may not ask (on macOS, without the
    /// Accessibility permission).
    Unknown,
}

/// The system's package manager, which installs the launcher's own deb, rpm or Arch package.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackageManager {
    Dnf,
    Apt,
    Pacman,
}

/// Why the package manager didn't install the launcher's package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackageError {
    /// The player closed the password prompt, or wasn't allowed.
    Cancelled,
    /// The package manager's own error.
    Failed(String),
}

/// The desktop's windows: which one is in front, and bringing one to the front. A game is
/// its processes, as [`ProcessTree::members`] lists them.
pub trait Windows: Send + Sync {
    fn front(&self) -> Front;
    /// Brings a window of one of the game's processes `ids` to the front. `false` when none has
    /// a window yet, or the system refused.
    fn show_game(&self, ids: &[u32]) -> bool;
    /// Brings the launcher's window to the front.
    fn show_launcher(&self);
    /// Whether one of the processes `ids` has a window. `None` when the system doesn't say.
    fn has_window(&self, _ids: &[u32]) -> Option<bool> {
        None
    }
    /// Whether a game whose windows all closed is over even while its processes run. True on
    /// macOS, where a program keeps running after its last window closes. Elsewhere a game
    /// ends when its processes do: a launcher's own window (a menu, a setup dialog) closes
    /// before the game's opens.
    fn window_close_ends_game(&self) -> bool {
        false
    }
    /// Why games can't come to the front on this desktop, and the fix, for example a missing
    /// tool or permission. `None` when nothing is missing. The core tells the player once a run.
    fn problem(&self) -> Option<String> {
        None
    }
}

/// Windows the launcher can't see or change. The default until the app plugs in real ones.
pub struct NoWindows;

impl Windows for NoWindows {
    fn front(&self) -> Front {
        Front::Unknown
    }

    fn show_game(&self, _ids: &[u32]) -> bool {
        false
    }

    fn show_launcher(&self) {}
}

/// The player's login session: starting the launcher when the player logs in, and the
/// administrator prompt that installs the launcher's own package. Not a play session.
pub trait LoginSession: Send + Sync {
    /// Whether the launcher can start when the player logs in. `false` for a build that isn't
    /// installed, such as `cargo run`'s: the system would start a program that moves.
    fn can_start_at_login(&self) -> bool {
        false
    }
    /// Starts the launcher when the player logs in (`on`), or stops that. The error says why
    /// the system refused.
    fn start_at_login(&self, _on: bool) -> Result<(), String> {
        Err("this desktop can't start the launcher when you log in".into())
    }
    /// Installs the launcher's own package `file` with `manager`, asking for the administrator
    /// password the system's way. Blocking: it returns once the package manager is done.
    fn install_package(&self, _file: &Path, _manager: PackageManager) -> Result<(), PackageError> {
        Err(PackageError::Failed("this system can't install packages".into()))
    }
}

/// A session that can't start the launcher at login or install packages. The default until the
/// app plugs in a real one.
pub struct NoLoginSession;

impl LoginSession for NoLoginSession {}

/// The system's opener: what a click on a link or a file does.
pub trait Opener: Send + Sync {
    /// Opens `link` with the system's handler for it: an `https://` page in the browser, and
    /// `steam://install/440` asks Steam to install a game, starting Steam on its own when it
    /// isn't running. The caller checked the link (the core against the plugin's
    /// `permissions.links`). The error says why it didn't open.
    fn open_link(&self, _link: &str) -> Result<(), String> {
        Err("this desktop can't open links".into())
    }
    /// Opens `path` with the system's opener: a folder in the file manager, a log in the text
    /// viewer. The error says why it didn't open.
    fn open_path(&self, _path: &Path) -> Result<(), String> {
        Err("this desktop can't open files".into())
    }
}

/// An opener that opens nothing. The default until the app plugs in a real one.
pub struct NoOpener;

impl Opener for NoOpener {}

/// A running game's processes: the one the launcher started, the ones in its process group,
/// and the ones a sandbox moved out of the group below them (Proton's pressure-vessel does).
/// The app's adapter walks the system's process list; the core's default sees only the group.
pub trait ProcessTree: Send + Sync {
    /// The live processes of the game the launcher started as `root`, `root` among them while
    /// it runs. Blocking: it may run a system tool.
    fn members(&self, root: u32) -> Vec<u32>;
    /// The processes whose command line has `text`, for a game another program started (see
    /// `LaunchSpec::game_process`): each one not below another of them. Blocking. None by
    /// default, where command lines can't be read.
    fn find(&self, _text: &str) -> Vec<u32> {
        Vec::new()
    }
    /// Asks the processes `ids` to close (`force`: kills them). Blocking.
    fn end(&self, ids: &[u32], force: bool);
}

/// The process group alone: `root` while its group has a live process (Unix), nothing on
/// Windows, which has no process groups. The default until the app plugs in a real one. Once
/// the first process exits the core drops `root`, so with this default a game ends with its
/// first process: plug in the app's adapter to follow the group.
pub struct GroupTree;

impl ProcessTree for GroupTree {
    fn members(&self, root: u32) -> Vec<u32> {
        #[cfg(unix)]
        {
            let alive = std::process::Command::new("kill")
                .args(["-0", "--", &format!("-{root}")])
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
            if alive { vec![root] } else { Vec::new() }
        }
        #[cfg(not(unix))]
        {
            let _ = root;
            Vec::new()
        }
    }

    fn end(&self, _ids: &[u32], _force: bool) {}
}

/// HTTP that always fails. The default until the app plugs in a real transport.
pub struct NoNetwork;

#[async_trait]
impl HttpTransport for NoNetwork {
    async fn send(&self, request: HttpRequest) -> PluginResult<HttpResponse> {
        Err(nexus_api::PluginError::Network(format!("no network transport for {}", request.url)))
    }

    async fn open(&self, url: &str, _offset: u64) -> PluginResult<Opened> {
        Err(nexus_api::PluginError::Network(format!("no network transport for {url}")))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// Real programs run in real time.
    const REAL: &dyn Clock = &TokioClock;

    fn sh(script: &str) -> SpawnCommand {
        SpawnCommand {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            env: BTreeMap::new(),
            cwd: std::env::temp_dir(),
            log: None,
            keep_running: false,
        }
    }

    /// A program that may become Steam's client outlives its handle, and so Nexus.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_program_that_may_outlive_nexus_isn_t_stopped_when_its_handle_goes() {
        let mut command = sh("sleep 30");
        command.keep_running = true;
        let process = TokioSpawner.spawn(command).await.unwrap();
        let id = process.id().unwrap().to_string();
        drop(process);
        TokioClock.sleep(Duration::from_millis(200)).await;
        let alive = std::process::Command::new("kill").args(["-0", &id]).status().is_ok_and(|s| s.success());
        let _ = std::process::Command::new("kill").args(["-KILL", &id]).status();
        assert!(alive, "the program was stopped with its handle");
    }

    #[test]
    fn a_command_reads_as_one_shell_line() {
        let command = SpawnCommand {
            program: "/Games/My Game/run.sh".into(),
            args: vec!["--fullscreen".into(), "it's".into(), String::new()],
            env: BTreeMap::from([("WINEPREFIX".to_owned(), "/data/prefixes/a b".to_owned())]),
            cwd: "/Games/My Game".into(),
            log: None,
            keep_running: false,
        };
        assert_eq!(
            command.shell_line(),
            r#"cd '/Games/My Game' && WINEPREFIX='/data/prefixes/a b' '/Games/My Game/run.sh' --fullscreen 'it'\''s' ''"#
        );
    }

    #[tokio::test]
    async fn output_reads_what_a_program_prints_up_to_the_limit() {
        let out = TokioSpawner.output(sh("echo hello"), Duration::from_secs(5), 1024).await.unwrap();
        assert_eq!(out.as_deref(), Some(&b"hello\n"[..]));
        // More than the limit: the read stops there, and the program stops on its closed pipe.
        let out = TokioSpawner.output(sh("yes"), Duration::from_secs(5), 10).await.unwrap();
        assert_eq!(out.map(|o| o.len()), Some(10));
    }

    #[tokio::test]
    async fn output_stops_a_program_that_runs_past_the_timeout() {
        let started = REAL.now();
        let out = TokioSpawner.output(sh("sleep 30"), Duration::from_millis(200), 1024).await.unwrap();
        assert_eq!(out, None);
        assert!(REAL.now() - started < Duration::from_secs(5));
    }

    async fn alive(pid: &str) -> bool {
        let status =
            tokio::process::Command::new("kill").args(["-0", pid]).stderr(std::process::Stdio::null()).status();
        status.await.unwrap().success()
    }

    #[tokio::test]
    async fn terminate_asks_a_program_to_close() {
        let mut process = TokioSpawner.spawn(sh("sleep 30")).await.unwrap();
        assert!(process.id().is_some());

        process.terminate().await.unwrap();

        let exit = REAL.timeout(Duration::from_secs(5), process.wait()).await.unwrap().unwrap();
        assert!(!exit.success);
    }

    #[tokio::test]
    async fn kill_stops_the_programs_a_game_started_too() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("child");
        // A start script that ignores the request to close and leaves a program running.
        let script = format!("trap '' TERM; sleep 30 & echo $! > {}; wait", pidfile.display());
        let mut process = TokioSpawner.spawn(sh(&script)).await.unwrap();
        let child = loop {
            match std::fs::read_to_string(&pidfile) {
                Ok(pid) if pid.ends_with('\n') => break pid.trim().to_owned(),
                _ => TokioClock.sleep(Duration::from_millis(20)).await,
            }
        };
        assert!(alive(&child).await);

        process.kill().await.unwrap();
        process.wait().await.unwrap();

        let gone = async {
            while alive(&child).await {
                TokioClock.sleep(Duration::from_millis(20)).await;
            }
        };
        REAL.timeout(Duration::from_secs(5), gone).await.expect("the script's program stopped too");
    }

    #[tokio::test]
    async fn a_program_s_output_goes_to_its_log() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("logs/game.log");
        let mut command = sh("echo out; echo err >&2");
        command.log = Some(log.clone());

        let mut process = TokioSpawner.spawn(command).await.unwrap();
        process.wait().await.unwrap();

        assert_eq!(std::fs::read_to_string(log).unwrap(), "out\nerr\n");
    }

    #[test]
    fn which_finds_a_program_on_the_path() {
        assert!(TokioSpawner.which("sh").is_some_and(|p| p.is_file()));
        assert_eq!(TokioSpawner.which("no-such-program-anywhere"), None);
    }
}

/// One graphics card.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gpu {
    pub name: String,
    /// Its own memory in GB; `None` for memory it shares with the processor, or unknown.
    pub memory_gb: Option<f32>,
    /// A card of its own, not one built into the processor.
    pub discrete: bool,
    /// Built in, and its memory is the computer's own (Apple Silicon): as fast as a card of
    /// its own can be, with the system's memory to use.
    pub unified: bool,
    /// The Vulkan version its driver supports, as (major, minor); `None` when unknown.
    pub vulkan: Option<(u32, u32)>,
}

/// This computer, as far as the system says. Unknown parts are empty or `None`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Specs {
    /// For example "AMD Ryzen 9 7950X".
    pub cpu: String,
    pub cores: u32,
    pub threads: u32,
    pub memory_gb: f32,
    /// The processor has AVX2; `None` where the system can't say.
    pub avx2: Option<bool>,
    /// `None` when the graphics check couldn't run.
    pub gpus: Option<Vec<Gpu>>,
}

/// What this computer has. Slow (it may run system tools): the core calls it off the UI
/// thread, once a run.
pub trait Hardware: Send + Sync {
    fn specs(&self) -> Specs;
    /// The free bytes on the disk that holds `path`, which must exist; `None` when the system
    /// doesn't say. Quick: one system call.
    fn free_bytes(&self, path: &std::path::Path) -> Option<u64>;
}

/// A computer the launcher can't look at. The default until the app plugs in a real one.
pub struct NoHardware;

impl Hardware for NoHardware {
    fn specs(&self) -> Specs {
        Specs::default()
    }

    fn free_bytes(&self, _: &std::path::Path) -> Option<u64> {
        None
    }
}

/// One file in a torrent, with `/` between the parts of its path.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TorrentFile {
    pub path: String,
    pub size: u64,
}

/// What a torrent holds, read from its metadata before any file downloads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TorrentInfo {
    pub name: String,
    pub files: Vec<TorrentFile>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TorrentPhase {
    /// Checking the pieces already in the folder.
    Checking,
    Downloading,
    /// Every piece is in, and it uploads to others.
    Seeding,
    /// Every piece is in, and it doesn't upload.
    Finished,
    /// It stopped with an error.
    Failed,
}

/// A torrent's state, as the transport sees it now.
#[derive(Clone, Debug, PartialEq)]
pub struct TorrentStatus {
    pub phase: TorrentPhase,
    /// Bytes checked or downloaded of `total`.
    pub done: u64,
    pub total: u64,
    /// Bytes a second.
    pub download_rate: u64,
    pub upload_rate: u64,
    pub peers: u32,
    /// For [`TorrentPhase::Failed`].
    pub error: String,
}

/// How a torrent starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TorrentStart {
    /// Downloads what is missing, then seeds if `seed` is on, else finishes.
    Download { seed: bool },
    /// Checks the pieces in the folder and seeds them. A missing piece fails it: nothing
    /// downloads.
    SeedOnly,
}

/// The upload limit while seeding, in bytes a second.
pub const SEED_UPLOAD_LIMIT: u64 = 128 * 1024;

/// BitTorrent. Torrents are named by their info hash, in lowercase hex.
#[async_trait]
pub trait Torrents: Send + Sync {
    /// Reads the torrent's name and files from the swarm. Downloads no file.
    async fn resolve(&self, magnet: &str) -> PluginResult<TorrentInfo>;
    /// Starts the torrent into `folder`, checking the pieces already there first.
    async fn start(&self, id: &str, magnet: &str, folder: &Path, how: TorrentStart) -> PluginResult<()>;
    /// Stops the torrent. Its files stay. No error when it isn't running.
    async fn stop(&self, id: &str) -> PluginResult<()>;
    /// `None` when it isn't running.
    fn status(&self, id: &str) -> Option<TorrentStatus>;
    /// The network interface that every connection uses from the next start; `None` for any.
    /// A change closes the connections the torrents shared, so the core stops them first.
    async fn route(&self, interface: Option<String>) -> PluginResult<()>;
    /// Stops every torrent and closes the connections they shared. Their files stay.
    async fn stop_all(&self) -> PluginResult<()>;
}

/// A host without BitTorrent. The default until the app plugs in a real one.
pub struct NoTorrents;

#[async_trait]
impl Torrents for NoTorrents {
    async fn resolve(&self, _magnet: &str) -> PluginResult<TorrentInfo> {
        Err(nexus_api::PluginError::Unsupported("this build of Nexus can't download torrents".into()))
    }
    async fn start(&self, _id: &str, _magnet: &str, _folder: &Path, _how: TorrentStart) -> PluginResult<()> {
        Err(nexus_api::PluginError::Unsupported("this build of Nexus can't download torrents".into()))
    }
    async fn stop(&self, _id: &str) -> PluginResult<()> {
        Ok(())
    }
    fn status(&self, _id: &str) -> Option<TorrentStatus> {
        None
    }
    async fn route(&self, _interface: Option<String>) -> PluginResult<()> {
        Ok(())
    }
    async fn stop_all(&self) -> PluginResult<()> {
        Ok(())
    }
}

/// A VPN, or on a system that can't tell VPNs from other networks, any network with an
/// address: the system plugins' type.
pub use nexus_api::Vpn;

/// The VPNs on this computer, for "Download only through a VPN". Each call can run a system
/// tool, so the core calls them off the UI thread.
pub trait Network: Send + Sync {
    /// Why downloads can't go only through a VPN here, for example "Not available on
    /// Windows". `None` when they can.
    fn problem(&self) -> Option<String> {
        None
    }
    /// The system tells VPNs from other networks. When it can't (macOS), [`Network::vpns`]
    /// lists every network with an address, and the player picks the one the VPN uses.
    fn finds_vpns(&self) -> bool {
        true
    }
    fn vpns(&self) -> Vec<Vpn>;
    /// The system can add a VPN from a provider's file.
    fn can_import(&self) -> bool {
        false
    }
    /// Adds a VPN from a provider's file (WireGuard `.conf`, OpenVPN `.ovpn`) and connects
    /// it. Returns its name.
    fn import(&self, _file: &Path) -> Result<String, String> {
        Err("this system can't add a VPN: connect it with the VPN's own app".into())
    }
    fn connect(&self, _name: &str) -> Result<(), String> {
        Err("this system can't connect a VPN: use the VPN's own app".into())
    }
    fn disconnect(&self, _name: &str) -> Result<(), String> {
        Err("this system can't disconnect a VPN: use the VPN's own app".into())
    }
}

/// A host that can't see its VPNs. The default until the app plugs in a real one.
pub struct NoVpn;

impl Network for NoVpn {
    fn problem(&self) -> Option<String> {
        Some("This build of Nexus can't find VPNs".into())
    }

    fn vpns(&self) -> Vec<Vpn> {
        Vec::new()
    }
}

/// What an archive set is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArchiveKind {
    Rar,
    #[serde(rename = "7z")]
    SevenZ,
    Zip,
    Tar,
    #[serde(rename = "tar.gz")]
    TarGz,
    #[serde(rename = "tar.xz")]
    TarXz,
    #[serde(rename = "tar.bz2")]
    TarBz2,
    #[serde(rename = "tar.zst")]
    TarZst,
    /// A disc image.
    Iso,
    /// A Windows cabinet.
    Cab,
    /// A GOG installer: a shell script with a zip at its end. The script never runs.
    Gog,
    /// A script installer that states its archive's size (Kron4ek's YAD installers): the
    /// archive is its last bytes, from [`crate::embedded_archive`]. The script never runs.
    Script,
}

/// How far an unpack is: `done` of `total` bytes written.
pub type UnpackProgress = Arc<dyn Fn(u64, u64) + Send + Sync>;

/// How an entry's path gets out of the unpack folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Escape {
    /// Its path is absolute or climbs out with `..`.
    Leaves,
    /// It is a link whose target is absolute or climbs out with `..`.
    LinksOut,
    /// A link already on the disk sits where it would go.
    ThroughLink,
    /// Once the links on the disk are followed, its path is outside the folder.
    LeavesThroughLink,
}

/// Why an archive didn't unpack. The core shows its text; an adapter picks a fallback reader
/// by its variant, never by its text.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ArchiveError {
    #[error("This archive needs a password, which Nexus can't ask for")]
    Password,
    /// An entry's name, as the archive spells it, and how it gets out of the folder.
    #[error("{}", escape_text(.entry, *.how))]
    Escapes { entry: String, how: Escape },
    /// The bytes the unpack needs, spare included, and the bytes the disk has.
    #[error("This archive needs {} free, and the disk has {}", gigabytes(*.needed), gigabytes(*.free))]
    NoRoom { needed: u64, free: u64 },
    /// The reader can't read this archive: a format, a feature or a codec it lacks. Another
    /// reader may.
    #[error("{0}")]
    Unsupported(String),
    /// A library or program the archive needs isn't installed. The text says what to install.
    #[error("{0}")]
    Missing(String),
    /// The archive is damaged, cut short, or changed while it was unpacked.
    #[error("{0}")]
    Corrupt(String),
    /// Reading the parts or writing the folder failed.
    #[error("{0}")]
    Io(String),
    #[error("The unpacking was cancelled")]
    Cancelled,
}

fn escape_text(entry: &str, how: Escape) -> String {
    match how {
        Escape::Leaves => format!("{entry} leaves the folder"),
        Escape::LinksOut => format!("{entry} links out of the folder"),
        Escape::ThroughLink => format!("{entry} would write through a link"),
        Escape::LeavesThroughLink => format!("{entry} leaves the folder through a link"),
    }
}

fn gigabytes(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

/// Unpacking archives. The work is blocking, so an adapter runs it off the async runtime.
///
/// An adapter lists the archive with its format readers, hands the listing to
/// [`crate::plan_unpack`], and writes only what the plan holds. The safety rules live in the
/// plan, so every adapter refuses the same archives.
#[async_trait]
pub trait Archives: Send + Sync {
    /// Unpacks the archive cut into `parts` (in order) into the empty folder `into`, on a disk
    /// with `free` bytes (`None` when the disk doesn't say). Before it writes anything it
    /// refuses an archive that needs a password, an entry whose path leaves `into` (an absolute
    /// path, `..`, or a link that points out), and an archive bigger than the free space. It
    /// stops with [`ArchiveError::Cancelled`] soon after `cancel` is set.
    async fn unpack(
        &self,
        kind: ArchiveKind,
        parts: &[PathBuf],
        into: &Path,
        free: Option<u64>,
        progress: UnpackProgress,
        cancel: Arc<AtomicBool>,
    ) -> Result<(), ArchiveError>;
}

/// A host that can't unpack archives. The default until the app plugs in a real one.
pub struct NoArchives;

#[async_trait]
impl Archives for NoArchives {
    async fn unpack(
        &self,
        _kind: ArchiveKind,
        _parts: &[PathBuf],
        _into: &Path,
        _free: Option<u64>,
        _progress: UnpackProgress,
        _cancel: Arc<AtomicBool>,
    ) -> Result<(), ArchiveError> {
        Err(ArchiveError::Unsupported("this build of Nexus can't unpack archives".into()))
    }
}

/// A system plugin's VPNs ([`nexus_api::VpnSettings`]) as the core's [`Network`] port: once one
/// provides them, the core reads and switches its VPNs through it.
pub(crate) struct PluginVpn(pub Arc<dyn nexus_api::VpnSettings>);

impl Network for PluginVpn {
    fn problem(&self) -> Option<String> {
        self.0.problem()
    }

    fn finds_vpns(&self) -> bool {
        self.0.finds_vpns()
    }

    fn vpns(&self) -> Vec<Vpn> {
        self.0.vpns()
    }

    fn can_import(&self) -> bool {
        self.0.can_import()
    }

    fn import(&self, file: &Path) -> Result<String, String> {
        self.0.import(file)
    }

    fn connect(&self, name: &str) -> Result<(), String> {
        self.0.connect(name)
    }

    fn disconnect(&self, name: &str) -> Result<(), String> {
        self.0.disconnect(name)
    }
}
