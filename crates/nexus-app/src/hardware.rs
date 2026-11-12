//! "This PC" on each system: the processor and memory from the system's own tools, the
//! graphics cards from the Vulkan probe in a child process.

use std::path::PathBuf;
use std::time::Duration;

use nexus_core::{Gpu, Hardware, Specs};

pub struct SystemHardware {
    /// Where the emulators' builds are, for the MoltenVK they carry on macOS.
    pub data_dir: PathBuf,
}

/// Apple Silicon: its GPU is built in and shares the computer's memory as its own.
pub const APPLE_SILICON: bool = cfg!(all(target_os = "macos", target_arch = "aarch64"));

/// How long the graphics probe may take: a hung driver must not hold the check up.
const PROBE_TIME: Duration = Duration::from_secs(10);

impl Hardware for SystemHardware {
    fn specs(&self) -> Specs {
        let mut specs = processor_and_memory();
        specs.avx2 = avx2();
        if specs.threads == 0 {
            specs.threads = std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(0);
        }
        specs.gpus = probe_gpus(&self.data_dir).or_else(named_gpus);
        specs
    }

    fn free_bytes(&self, path: &std::path::Path) -> Option<u64> {
        crate::disk::free_bytes(path)
    }

    fn computer_name(&self) -> String {
        computer_name()
    }

    fn disk_bytes(&self, path: &std::path::Path) -> Option<u64> {
        crate::disk::disk_bytes(path)
    }

    fn controllers(&self) -> Vec<nexus_api::Controller> {
        let exe = match std::env::current_exe() {
            Ok(exe) => exe,
            Err(error) => {
                tracing::warn!(%error, "no controllers: the launcher's own path is unknown; games use the keyboard");
                return Vec::new();
            }
        };
        let mut probe = std::process::Command::new(exe);
        probe.arg("--probe-controllers");
        // A controller probe that gives nothing leaves games on the keyboard, so say why.
        match run_probe(probe, CONTROLLER_PROBE_TIME) {
            ProbeRun::Printed(text) => {
                let found = crate::controller_probe::parse(&text);
                if found.is_empty() {
                    tracing::info!("the controller probe found no controllers; games use the keyboard");
                }
                found
            }
            ProbeRun::NoStart(error) => {
                tracing::warn!(
                    %error,
                    "the controller probe did not start; games use the keyboard: restart the launcher after a rebuild"
                );
                Vec::new()
            }
            ProbeRun::Failed(code) => {
                tracing::warn!(?code, "the controller probe failed; games use the keyboard");
                Vec::new()
            }
            ProbeRun::TooSlow => {
                tracing::warn!("the controller probe took too long; games use the keyboard");
                Vec::new()
            }
        }
    }
}

/// This computer's name on the network; empty when the system doesn't say.
#[allow(unsafe_code)] // One system call that fills a buffer this function owns.
pub fn computer_name() -> String {
    #[cfg(unix)]
    {
        let mut buf = [0u8; 256];
        // SAFETY: `buf` is writable for its length; the name ends with a nul within it.
        if unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len() - 1) } != 0 {
            return String::new();
        }
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        String::from_utf8_lossy(&buf[..end]).into_owned()
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::SystemInformation::{ComputerNamePhysicalDnsHostname, GetComputerNameExW};
        let mut buf = [0u16; 256];
        let mut len = buf.len() as u32;
        // SAFETY: `buf` is writable for `len` wide characters.
        if unsafe { GetComputerNameExW(ComputerNamePhysicalDnsHostname, buf.as_mut_ptr(), &mut len) } == 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..len as usize])
    }
    #[cfg(not(any(unix, windows)))]
    String::new()
}

/// How long the controller probe may take: SDL waits half a second for controllers to show up.
const CONTROLLER_PROBE_TIME: Duration = Duration::from_secs(5);

/// The processor has AVX2, from the processor itself; unknown on other processors.
fn avx2() -> Option<bool> {
    #[cfg(target_arch = "x86_64")]
    return Some(std::arch::is_x86_feature_detected!("avx2"));
    #[cfg(not(target_arch = "x86_64"))]
    None
}

fn output(program: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(program).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Runs `nexus --probe-gpu`, at most `PROBE_TIME`. `None` when no Vulkan loader answered.
fn probe_gpus(data_dir: &std::path::Path) -> Option<Vec<Gpu>> {
    let mut probe = std::process::Command::new(std::env::current_exe().ok()?);
    // MoltenVK writes its log to standard output.
    probe.arg("--probe-gpu").env("MVK_CONFIG_LOG_LEVEL", "0").arg("--data-dir").arg(data_dir);
    match run_probe(probe, PROBE_TIME) {
        ProbeRun::Printed(text) => Some(crate::gpu_probe::parse(&text)),
        ProbeRun::NoStart(error) => {
            tracing::warn!(%error, "the graphics probe did not start; the cards come from the system instead");
            None
        }
        ProbeRun::Failed(code) => {
            tracing::warn!(?code, "the graphics probe failed; the cards come from the system instead");
            None
        }
        ProbeRun::TooSlow => {
            tracing::warn!("the graphics probe took too long; the cards come from the system instead");
            None
        }
    }
}

/// What a probe child did. Each way it can go wrong is kept apart, so a probe that gives
/// nothing leaves a reason in the log instead of a silent fall back.
enum ProbeRun {
    /// It exited cleanly and printed this.
    Printed(String),
    /// It never started: the program is missing, or the system refused to run it.
    NoStart(std::io::Error),
    /// It ran and exited without success, with this exit code if it had one.
    Failed(Option<i32>),
    /// It was still running at the time limit.
    TooSlow,
}

/// Runs a probe child process for at most `time`, and says what it did.
fn run_probe(mut probe: std::process::Command, time: Duration) -> ProbeRun {
    let mut child = match probe.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn() {
        Ok(child) => child,
        Err(error) => return ProbeRun::NoStart(error),
    };
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return ProbeRun::Failed(status.code());
                }
                let mut text = String::new();
                let Some(mut out) = child.stdout.take() else { return ProbeRun::Failed(status.code()) };
                return match std::io::Read::read_to_string(&mut out, &mut text) {
                    Ok(_) => ProbeRun::Printed(text),
                    Err(error) => ProbeRun::NoStart(error),
                };
            }
            Ok(None) if started.elapsed() < time => std::thread::sleep(Duration::from_millis(50)),
            Ok(None) => {
                let _ = child.kill();
                return ProbeRun::TooSlow;
            }
            Err(error) => {
                let _ = child.kill();
                return ProbeRun::NoStart(error);
            }
        }
    }
}

/// Without Vulkan: the cards' names the system gives, with their Vulkan version unknown.
fn named_gpus() -> Option<Vec<Gpu>> {
    if cfg!(target_os = "macos") {
        return output("system_profiler", &["SPDisplaysDataType", "-json"]).map(|text| parse_system_profiler(&text));
    }
    None
}

fn processor_and_memory() -> Specs {
    if cfg!(target_os = "macos") {
        let text =
            output("sysctl", &["-n", "machdep.cpu.brand_string", "hw.physicalcpu", "hw.logicalcpu", "hw.memsize"]);
        return text.map(|t| parse_sysctl(&t)).unwrap_or_default();
    }
    if cfg!(windows) {
        let script = "Get-CimInstance Win32_Processor | Select-Object Name,NumberOfCores,NumberOfLogicalProcessors | ConvertTo-Json -Compress; \
                      (Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory";
        let text = output("powershell", &["-NoProfile", "-NonInteractive", "-Command", script]);
        return text.map(|t| parse_windows(&t)).unwrap_or_default();
    }
    let cpu = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let memory = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    parse_linux(&cpu, &memory)
}

const GIB: f32 = (1u64 << 30) as f32;

/// `sysctl -n machdep.cpu.brand_string hw.physicalcpu hw.logicalcpu hw.memsize`: one value
/// per line.
pub fn parse_sysctl(text: &str) -> Specs {
    let mut lines = text.lines().map(str::trim);
    let cpu = lines.next().unwrap_or_default().to_owned();
    let cores = lines.next().and_then(|l| l.parse().ok()).unwrap_or(0);
    let threads = lines.next().and_then(|l| l.parse().ok()).unwrap_or(0);
    let memory_gb = lines.next().and_then(|l| l.parse::<u64>().ok()).map_or(0.0, |b| b as f32 / GIB);
    Specs { cpu, cores, threads, memory_gb, ..Specs::default() }
}

/// `system_profiler SPDisplaysDataType -json`: the cards' names, and the memory of a card of
/// its own ("spdisplays_vram": "8 GB").
pub fn parse_system_profiler(text: &str) -> Vec<Gpu> {
    let Ok(json) = nexus_api::serde_json::from_str::<nexus_api::serde_json::Value>(text) else { return Vec::new() };
    json["SPDisplaysDataType"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|card| {
            let discrete = card["sppci_bus"].as_str().is_some_and(|bus| bus != "spdisplays_builtin");
            let memory_gb = card["spdisplays_vram"]
                .as_str()
                .or(card["spdisplays_vram_shared"].as_str().filter(|_| discrete))
                .and_then(|v| v.split_whitespace().next()?.parse::<f32>().ok())
                .filter(|_| discrete);
            Gpu {
                name: card["sppci_model"].as_str().or(card["_name"].as_str()).unwrap_or_default().to_owned(),
                memory_gb,
                discrete,
                unified: !discrete && APPLE_SILICON,
                vulkan: None,
            }
        })
        .collect()
}

/// `/proc/cpuinfo` and `/proc/meminfo`.
pub fn parse_linux(cpuinfo: &str, meminfo: &str) -> Specs {
    let field = |line: &str, name: &str| {
        let (key, value) = line.split_once(':')?;
        (key.trim() == name).then(|| value.trim().to_owned())
    };
    let cpu = cpuinfo.lines().find_map(|l| field(l, "model name")).unwrap_or_default();
    let threads = cpuinfo.lines().filter(|l| field(l, "processor").is_some()).count() as u32;
    let mut cores = std::collections::BTreeSet::new();
    let mut socket = String::new();
    for line in cpuinfo.lines() {
        if let Some(id) = field(line, "physical id") {
            socket = id;
        }
        if let Some(core) = field(line, "core id") {
            cores.insert((socket.clone(), core));
        }
    }
    let memory_gb = meminfo
        .lines()
        .find_map(|l| field(l, "MemTotal"))
        .and_then(|v| v.split_whitespace().next()?.parse::<u64>().ok())
        .map_or(0.0, |kb| (kb * 1024) as f32 / GIB);
    let cores = if cores.is_empty() { threads } else { cores.len() as u32 };
    Specs { cpu, cores, threads, memory_gb, ..Specs::default() }
}

/// The PowerShell lines: the processor as JSON (one object, or a list for several sockets),
/// then the memory in bytes.
pub fn parse_windows(text: &str) -> Specs {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let processors = lines
        .next()
        .and_then(|l| nexus_api::serde_json::from_str::<nexus_api::serde_json::Value>(l).ok())
        .unwrap_or_default();
    let list = match processors {
        nexus_api::serde_json::Value::Array(list) => list,
        one => vec![one],
    };
    let count = |key: &str| list.iter().filter_map(|p| p[key].as_u64()).sum::<u64>() as u32;
    let memory_gb = lines.next().and_then(|l| l.parse::<u64>().ok()).map_or(0.0, |b| b as f32 / GIB);
    Specs {
        cpu: list.first().and_then(|p| p["Name"].as_str()).unwrap_or_default().trim().to_owned(),
        cores: count("NumberOfCores"),
        threads: count("NumberOfLogicalProcessors"),
        memory_gb,
        ..Specs::default()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn this_computer_has_a_name() {
        assert!(!super::computer_name().is_empty());
    }

    use super::*;

    #[test]
    fn a_mac_reads_from_sysctl_and_system_profiler() {
        // This Mac's own output.
        let specs = parse_sysctl("Apple M4 Max\n14\n14\n38654705664\n");
        assert_eq!((specs.cpu.as_str(), specs.cores, specs.threads, specs.memory_gb), ("Apple M4 Max", 14, 14, 36.0));
        let gpus = parse_system_profiler(
            r#"{ "SPDisplaysDataType" : [ { "_name" : "Apple M4 Max", "sppci_bus" : "spdisplays_builtin", "sppci_cores" : "32", "sppci_model" : "Apple M4 Max" } ] }"#,
        );
        assert_eq!(
            gpus,
            [Gpu {
                name: "Apple M4 Max".into(),
                memory_gb: None,
                discrete: false,
                unified: APPLE_SILICON,
                vulkan: None
            }]
        );
        // An Intel Mac's card of its own; the shape is system_profiler's documented one.
        let gpus = parse_system_profiler(
            r#"{ "SPDisplaysDataType" : [ { "sppci_bus" : "spdisplays_pcie_device", "sppci_model" : "AMD Radeon Pro 5500M", "spdisplays_vram" : "8 GB" } ] }"#,
        );
        assert_eq!(gpus[0].memory_gb, Some(8.0));
        assert!(gpus[0].discrete);
    }

    #[test]
    fn linux_reads_proc() {
        // Written in /proc's documented shape: two cores with two threads each.
        let cpuinfo = "processor\t: 0\nmodel name\t: AMD Ryzen 5 5600X\nphysical id\t: 0\ncore id\t: 0\n\n\
                       processor\t: 1\nmodel name\t: AMD Ryzen 5 5600X\nphysical id\t: 0\ncore id\t: 0\n\n\
                       processor\t: 2\nmodel name\t: AMD Ryzen 5 5600X\nphysical id\t: 0\ncore id\t: 1\n\n\
                       processor\t: 3\nmodel name\t: AMD Ryzen 5 5600X\nphysical id\t: 0\ncore id\t: 1\n";
        let specs = parse_linux(cpuinfo, "MemTotal:       16777216 kB\nMemFree: 1 kB\n");
        assert_eq!(
            (specs.cpu.as_str(), specs.cores, specs.threads, specs.memory_gb),
            ("AMD Ryzen 5 5600X", 2, 4, 16.0)
        );
    }

    #[test]
    fn windows_reads_powershell() {
        // Written in ConvertTo-Json's documented shape.
        let text = "{\"Name\":\"Intel(R) Core(TM) i7-12700K  \",\"NumberOfCores\":12,\"NumberOfLogicalProcessors\":20}\r\n34359738368\r\n";
        let specs = parse_windows(text);
        assert_eq!(
            (specs.cpu.as_str(), specs.cores, specs.threads, specs.memory_gb),
            ("Intel(R) Core(TM) i7-12700K", 12, 20, 32.0)
        );
    }

    // The probe outcomes, so a silent keyboard-only launch leaves a reason in the log.
    #[cfg(unix)]
    mod probe {
        use std::process::Command;
        use std::time::Duration;

        use super::super::{ProbeRun, run_probe};

        const LONG: Duration = Duration::from_secs(5);

        #[test]
        fn a_clean_exit_gives_what_it_printed() {
            let mut echo = Command::new("printf");
            echo.arg("one\ntwo\n");
            assert!(matches!(run_probe(echo, LONG), ProbeRun::Printed(text) if text == "one\ntwo\n"));
        }

        #[test]
        fn a_non_zero_exit_keeps_the_code() {
            let mut fails = Command::new("sh");
            fails.args(["-c", "exit 3"]);
            assert!(matches!(run_probe(fails, LONG), ProbeRun::Failed(Some(3))));
        }

        #[test]
        fn a_program_that_isn_t_there_doesn_t_start() {
            let missing = Command::new("nexus-no-such-probe-program");
            assert!(matches!(run_probe(missing, LONG), ProbeRun::NoStart(_)));
        }

        #[test]
        fn a_program_past_the_limit_is_too_slow() {
            let mut slow = Command::new("sleep");
            slow.arg("5");
            assert!(matches!(run_probe(slow, Duration::from_millis(100)), ProbeRun::TooSlow));
        }
    }
}
