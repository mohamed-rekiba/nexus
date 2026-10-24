//! "This PC": whether an emulator's games run on this computer, from what its manifest
//! requires (`[requires]`) and what the system says it has.

use nexus_api::Requirements;
use nexus_ui_api::{HardwareVm, VerdictVm};

use crate::{Gpu, Specs};

/// How well a plugin's games run here.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Verdict {
    Good,
    /// Runs, but slower than the games need, and the main reason ("Integrated GPU").
    Slow(String),
    /// Won't start, and why ("No Vulkan 1.3").
    No(String),
    /// The system didn't say enough, and what would let the launcher check.
    Unknown(String),
}

/// The Vulkan version in a manifest, `"1.3"`, as (major, minor).
fn version(text: &str) -> Option<(u32, u32)> {
    let (major, minor) = text.split_once('.').unwrap_or((text, "0"));
    Some((major.trim().parse().ok()?, minor.trim().parse().ok()?))
}

/// The card games will use: the one that meets the Vulkan version, then a card of its own,
/// then the most memory.
fn best_gpu(gpus: &[Gpu], vulkan: Option<(u32, u32)>) -> Option<&Gpu> {
    let key = |g: &Gpu| {
        (vulkan.is_none_or(|v| g.vulkan.is_some_and(|have| have >= v)), g.discrete, g.memory_gb.unwrap_or(0.0))
    };
    gpus.iter().max_by(|a, b| key(a).partial_cmp(&key(b)).unwrap_or(std::cmp::Ordering::Equal))
}

pub(crate) fn verdict(specs: &Specs, requires: &Requirements) -> Verdict {
    let Some(gpus) = &specs.gpus else {
        return Verdict::Unknown("Can't read the graphics cards".into());
    };
    let vulkan = requires.vulkan.as_deref().and_then(version);
    let Some(gpu) = best_gpu(gpus, vulkan) else {
        return Verdict::No("No graphics card found".into());
    };
    if let Some(need) = vulkan {
        match gpu.vulkan {
            Some(have) if have < need => return Verdict::No(format!("No Vulkan {}.{}", need.0, need.1)),
            Some(_) => {}
            None => return Verdict::Unknown("Can't read the graphics card's Vulkan version".into()),
        }
    }
    if requires.avx2 && specs.avx2 == Some(false) {
        return Verdict::No("No AVX2 in the processor".into());
    }
    let rec = &requires.recommended;
    // A unified GPU uses the system's memory; macOS lets it have about three quarters.
    let gpu_memory = if gpu.unified { Some(specs.memory_gb * 0.75) } else { gpu.memory_gb };
    let slow = if rec.discrete_gpu && !gpu.discrete && !gpu.unified {
        Some("Integrated GPU".to_owned())
    } else if let (Some(need), Some(have)) = (rec.gpu_memory_gb, gpu_memory)
        && have < need
    {
        Some(format!("{} GB GPU", have.round()))
    } else if rec.cpu_threads.is_some_and(|need| specs.threads < need) {
        Some(format!("{}-thread CPU", specs.threads))
    } else if rec.memory_gb.is_some_and(|need| specs.memory_gb < need) {
        Some(format!("{} GB RAM", specs.memory_gb.round()))
    } else {
        None
    };
    slow.map_or(Verdict::Good, Verdict::Slow)
}

/// The view of one emulator's verdict: `(id, name)` and the verdict.
pub(crate) fn verdict_vm(plugin: String, emulator: String, verdict: Verdict) -> VerdictVm {
    let (level, headline, why) = match verdict {
        Verdict::Good => ("good", "Should run well", String::new()),
        Verdict::Slow(why) => ("slow", "May run slowly", why),
        Verdict::No(why) => ("no", "Won't start", why),
        Verdict::Unknown(why) => ("unknown", "Can't tell yet", why),
    };
    VerdictVm { plugin, emulator, level: level.into(), headline: headline.into(), why }
}

/// The notice for the emulators, `(name, why)`, whose games won't start on this PC, such as
/// "This PC can't run PS5 games: No Vulkan 1.3"; `None` when there are none.
pub(crate) fn blocked_notice(blocked: &[(String, String)]) -> Option<String> {
    let names: Vec<&str> = blocked.iter().map(|(name, _)| name.as_str()).collect();
    let mut whys: Vec<&str> = blocked.iter().map(|(_, why)| why.as_str()).collect();
    whys.dedup();
    (!names.is_empty()).then(|| format!("This PC can't run {} games: {}", names.join(" or "), whys.join(", ")))
}

/// This PC on one line, for the config to keep: a new line is a changed PC.
pub(crate) fn summary(specs: &Specs) -> String {
    let gpus = specs.gpus.iter().flatten().map(|g| format!("{} {:?} {:?}", g.name, g.vulkan, g.memory_gb));
    let parts = [specs.cpu.clone(), specs.threads.to_string(), gb(specs.memory_gb), format!("{:?}", specs.avx2)];
    parts.into_iter().chain(gpus).collect::<Vec<_>>().join(" · ")
}

fn gb(value: f32) -> String {
    format!("{} GB", value.round())
}

/// The view of this PC and one verdict per emulator: `(id, name, requirements)`.
pub(crate) fn hardware_vm<'a>(
    specs: Option<&Specs>,
    emulators: impl Iterator<Item = (String, String, &'a Requirements)>,
) -> HardwareVm {
    let Some(specs) = specs else { return HardwareVm::default() };
    let cpu = match (specs.cores, specs.threads) {
        (0, _) => specs.cpu.clone(),
        (c, t) if c == t || t == 0 => format!("{} · {c} cores", specs.cpu),
        (c, t) => format!("{} · {c} cores, {t} threads", specs.cpu),
    };
    let graphics = specs
        .gpus
        .iter()
        .flatten()
        .map(|g| {
            let memory = g.memory_gb.map_or_else(|| "shared memory".to_owned(), gb);
            match g.vulkan {
                Some((major, minor)) => format!("{} · {memory} · Vulkan {major}.{minor}", g.name),
                None => format!("{} · {memory}", g.name),
            }
        })
        .collect();
    let verdicts =
        emulators.map(|(plugin, emulator, requires)| verdict_vm(plugin, emulator, verdict(specs, requires))).collect();
    HardwareVm {
        known: true,
        cpu,
        memory: if specs.memory_gb > 0.0 { gb(specs.memory_gb) } else { String::new() },
        graphics,
        verdicts,
    }
}
