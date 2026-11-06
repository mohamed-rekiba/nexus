//! A running game's processes, from the system's process list: the one the launcher started,
//! its process group, and every process below them. A sandbox starts its programs in a session
//! of their own, out of the group (Proton's pressure-vessel does); they are still below the
//! game's processes. Windows has no process groups: there a game is its process and the
//! processes below it, from a process snapshot.
//!
//! - Unix: `pgrep -g` lists the group, `ps -A -o pid=,ppid=` the parents; `kill` ends them. A
//!   game another program started is found by its command line, from `ps`.
//! - Windows: a ToolHelp snapshot lists the parents; `taskkill` ends them. It has no command
//!   lines, so a game another program started isn't found there.

use nexus_core::ProcessTree;

/// The system's processes.
pub struct SystemProcesses;

impl ProcessTree for SystemProcesses {
    fn members(&self, root: u32) -> Vec<u32> {
        #[cfg(unix)]
        {
            let mut ids = group_members(root);
            if is_running(root) && !ids.contains(&root) {
                ids.insert(0, root);
            }
            and_below(ids, &parents())
        }
        #[cfg(windows)]
        {
            let parents = system::parents();
            if !parents.iter().any(|(id, _)| *id == root) {
                return Vec::new();
            }
            and_below(vec![root], &parents)
        }
    }

    fn find(&self, text: &str) -> Vec<u32> {
        #[cfg(unix)]
        {
            let out = run("ps", &["-A", "-ww", "-o", "pid=,ppid=,args="]).unwrap_or_default();
            topmost(&with_command_lines(&out), text)
        }
        #[cfg(windows)]
        {
            let _ = text;
            Vec::new()
        }
    }

    fn end(&self, ids: &[u32], force: bool) {
        if ids.is_empty() {
            return;
        }
        let ids: Vec<String> = ids.iter().map(u32::to_string).collect();
        #[cfg(unix)]
        let status = std::process::Command::new("kill")
            .arg(if force { "-KILL" } else { "-TERM" })
            .arg("--")
            .args(&ids)
            .stderr(std::process::Stdio::null())
            .status();
        #[cfg(windows)]
        let status = std::process::Command::new("taskkill")
            .args(ids.iter().flat_map(|id| ["/PID", id.as_str()]))
            .args(if force { &["/F"][..] } else { &[] })
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        // A failed status means some were gone already, which is what we wanted.
        if let Err(e) = status {
            tracing::warn!(error = %e, "could not end the game's processes");
        }
    }
}

/// What a helper program printed, or `None` when it failed or isn't there.
#[cfg(unix)]
fn run(program: &str, args: &[&str]) -> Option<String> {
    use std::process::{Command, Stdio};
    let out = Command::new(program).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Whether process `id` runs.
#[cfg(unix)]
fn is_running(id: u32) -> bool {
    run("kill", &["-0", &id.to_string()]).is_some()
}

/// Running processes in the process group `group`. A game leads a group of its own, so these
/// are the programs it started; they keep the group after it exits.
#[cfg(unix)]
fn group_members(group: u32) -> Vec<u32> {
    run("pgrep", &["-g", &group.to_string()])
        .map(|out| out.split_whitespace().filter_map(|p| p.parse().ok()).collect())
        .unwrap_or_default()
}

/// Each running process with its parent, as `ps` lists them.
#[cfg(unix)]
fn parents() -> Vec<(u32, u32)> {
    run("ps", &["-A", "-o", "pid=,ppid="])
        .map(|out| {
            out.lines()
                .filter_map(|line| {
                    let mut ids = line.split_whitespace().map(|n| n.parse().ok());
                    Some((ids.next()??, ids.next()??))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Each process in `ps`'s `pid ppid args` lines, with its parent and command line.
#[cfg(unix)]
fn with_command_lines(ps: &str) -> Vec<(u32, u32, &str)> {
    ps.lines()
        .filter_map(|line| {
            let (id, rest) = line.trim_start().split_once(char::is_whitespace)?;
            let rest = rest.trim_start();
            let (parent, args) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
            Some((id.parse().ok()?, parent.parse().ok()?, args.trim_start()))
        })
        .collect()
}

/// Of the processes whose command line has `text`, those whose parent's hasn't: each game's
/// first process, not one it started with the same words.
#[cfg(any(unix, test))]
fn topmost(processes: &[(u32, u32, &str)], text: &str) -> Vec<u32> {
    let matches: Vec<_> = processes.iter().filter(|(_, _, args)| args.contains(text)).collect();
    matches
        .iter()
        .filter(|(_, parent, _)| !matches.iter().any(|(id, _, _)| id == parent))
        .map(|(id, _, _)| *id)
        .collect()
}

/// `ids` and every process below them, from `parents` (each process with its parent).
fn and_below(mut ids: Vec<u32>, parents: &[(u32, u32)]) -> Vec<u32> {
    let mut i = 0;
    while i < ids.len() {
        let parent = ids[i];
        for (child, of) in parents {
            // A process can be its own "parent" after its parent's id was reused.
            if *of == parent && *child != parent && !ids.contains(child) {
                ids.push(*child);
            }
        }
        i += 1;
    }
    ids
}

#[cfg(windows)]
#[allow(unsafe_code)] // ToolHelp is a foreign function; the snapshot is wrapped in a safe function here.
mod system {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    };

    /// Process id to parent process id, for every running process.
    pub fn parents() -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        // SAFETY: the snapshot handle is checked, read with a sized entry, and closed.
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return out;
            }
            let mut entry = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
            let mut more = Process32FirstW(snapshot, &mut entry) != 0;
            while more {
                out.push((entry.th32ProcessID, entry.th32ParentProcessID));
                more = Process32NextW(snapshot, &mut entry) != 0;
            }
            CloseHandle(snapshot);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use nexus_core::ProcessTree;

    use super::SystemProcesses;

    /// A game's processes: its own, and those below them, also below one in a session of its
    /// own, as Proton's sandbox starts the game.
    #[test]
    fn a_game_is_its_processes_and_every_one_below_them() {
        let parents = [(10, 1), (11, 10), (20, 11), (30, 20), (40, 1), (50, 50)];
        assert_eq!(super::and_below(vec![10, 11], &parents), [10, 11, 20, 30]);
        assert_eq!(super::and_below(vec![50], &parents), [50], "not its own child");
    }

    /// Of a game's processes that share its words, the first: Steam's reaper, not what it
    /// started with them.
    #[test]
    fn a_game_another_program_started_is_its_first_process_with_the_words() {
        let processes = [
            (5, 1, "steam steam://rungameid/570"),
            (40, 1, "reaper SteamLaunch AppId=570 -- game"),
            (41, 40, "game --from SteamLaunch AppId=570 -- game"),
            (60, 1, "reaper SteamLaunch AppId=5700 -- other"),
        ];
        assert_eq!(super::topmost(&processes, "SteamLaunch AppId=570 "), [40]);
        assert!(super::topmost(&processes, "SteamLaunch AppId=9 ").is_empty());
    }

    /// `ps` lines: the id, the parent and the command line, spaces in it kept.
    #[cfg(unix)]
    #[test]
    fn ps_lines_give_each_process_its_parent_and_command_line() {
        let ps = "    1     0 /sbin/init\n  4242  1  reaper SteamLaunch AppId=570 -- a b\n   77    1\n";
        assert_eq!(
            super::with_command_lines(ps),
            [(1, 0, "/sbin/init"), (4242, 1, "reaper SteamLaunch AppId=570 -- a b"), (77, 1, "")]
        );
    }

    /// The real process list finds a program by words in its command line.
    #[cfg(unix)]
    #[test]
    fn a_running_program_is_found_by_its_command_line() {
        // Words of this run only: a program an earlier run left can't match.
        let words = format!("29.{}", std::process::id());
        let mut program = std::process::Command::new("sleep").arg(&words).spawn().unwrap();
        let id = program.id();
        let found = || SystemProcesses.find(&format!("sleep {words}")) == [id];
        // The program is killed whatever the check says, and the check is asserted after.
        let start = Instant::now();
        let mut seen = found();
        while !seen && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(20));
            seen = found();
        }
        program.kill().unwrap();
        program.wait().unwrap();
        assert!(seen, "sleep {words} ({id}) wasn't found");
    }

    fn until(what: &str, done: impl Fn() -> bool) {
        let start = Instant::now();
        while !done() {
            assert!(start.elapsed() < Duration::from_secs(5), "timed out waiting until {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// A start script that starts the game in the background and exits: the game is still a
    /// member, in the script's group, until it is ended.
    #[cfg(unix)]
    #[test]
    fn a_program_a_script_left_in_the_background_is_still_the_game_s() {
        let mut script = std::process::Command::new("/bin/sh");
        script.args(["-c", "sleep 30 >/dev/null 2>&1 & echo $!"]).stdout(std::process::Stdio::piped());
        std::os::unix::process::CommandExt::process_group(&mut script, 0);
        let out = script.output().unwrap();
        let root: u32 = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap();
        // The script is gone; its group is the game's. Its leader's id is the group's.
        let group = {
            let out = std::process::Command::new("ps").args(["-o", "pgid=", "-p", &root.to_string()]).output().unwrap();
            String::from_utf8_lossy(&out.stdout).trim().parse::<u32>().unwrap()
        };
        let members = SystemProcesses.members(group);
        assert!(members.contains(&root), "the background program: {members:?}");

        SystemProcesses.end(&members, false);
        until("the game is gone", || SystemProcesses.members(group).is_empty());
    }
}
