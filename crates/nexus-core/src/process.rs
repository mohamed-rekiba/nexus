//! The real adapters that start and follow programs: [`TokioSpawner`] runs them with
//! `tokio::process`, and [`GroupTree`] follows a game by its process group. The traits they fill
//! are in [`crate::ports`]; the app's `SystemProcesses` is the fuller [`ProcessTree`].

use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use nexus_api::PluginResult;

use crate::ports::{Clock, ProcessExit, ProcessHandle, ProcessSpawner, ProcessTree, SpawnCommand, TokioClock};

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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

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
