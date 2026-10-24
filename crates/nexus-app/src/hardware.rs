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
}

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
    let exe = std::env::current_exe().ok()?;
    let mut child = std::process::Command::new(exe)
        .arg("--probe-gpu")
        // MoltenVK writes its log to standard output.
        .env("MVK_CONFIG_LOG_LEVEL", "0")
        .arg("--data-dir")
        .arg(data_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                let mut text = String::new();
                std::io::Read::read_to_string(&mut child.stdout.take()?, &mut text).ok()?;
                return Some(crate::gpu_probe::parse(&text));
            }
            Ok(None) if started.elapsed() < PROBE_TIME => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                return None;
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
}
