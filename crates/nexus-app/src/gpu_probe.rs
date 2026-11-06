//! The graphics cards, from Vulkan (the API both emulators draw with). It runs in a child
//! process (`nexus --probe-gpu`), so a broken driver can't take the launcher down, and prints
//! one line per card: `gpu<TAB>name<TAB>major.minor<TAB>discrete<TAB>memory GB or -<TAB>unified`.

use std::path::{Path, PathBuf};

use nexus_core::Gpu;

/// Where to look for a Vulkan loader: the system's, then (on macOS, which has none by
/// default) the MoltenVK the installed emulator builds carry.
fn loaders(data_dir: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = if cfg!(windows) {
        vec!["vulkan-1.dll".into()]
    } else if cfg!(target_os = "macos") {
        vec!["libvulkan.dylib".into(), "libvulkan.1.dylib".into(), "libMoltenVK.dylib".into()]
    } else {
        vec!["libvulkan.so.1".into()]
    };
    if cfg!(target_os = "macos") {
        found.extend(bundled_molten_vk(&data_dir.join("plugins")));
    }
    found
}

/// `libMoltenVK.dylib` under the emulators' build folders, a few folders down.
fn bundled_molten_vk(plugins: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut todo = vec![(plugins.to_path_buf(), 0)];
    while let Some((dir, depth)) = todo.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.file_name().is_some_and(|n| n == "libMoltenVK.dylib") {
                found.push(path);
            } else if depth < 7 && entry.file_type().is_ok_and(|t| t.is_dir()) {
                todo.push((path, depth + 1));
            }
        }
    }
    found
}

/// Prints the cards this process sees, for the parent to read. Returns false when no loader
/// worked.
pub fn print(data_dir: &Path) -> bool {
    for loader in loaders(data_dir) {
        if let Some(gpus) = probe(&loader) {
            for g in gpus {
                let (major, minor) = g.vulkan.unwrap_or((0, 0));
                let memory = g.memory_gb.map_or_else(|| "-".to_owned(), |m| format!("{m:.1}"));
                println!(
                    "gpu\t{}\t{major}.{minor}\t{}\t{memory}\t{}",
                    g.name.replace('\t', " "),
                    u8::from(g.discrete),
                    u8::from(g.unified)
                );
            }
            return true;
        }
    }
    false
}

/// The probe's lines, read back.
pub fn parse(text: &str) -> Vec<Gpu> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            if parts.next()? != "gpu" {
                return None;
            }
            let name = parts.next()?.to_owned();
            let (major, minor) = parts.next()?.split_once('.')?;
            let discrete = parts.next()? == "1";
            let memory_gb = parts.next()?.parse().ok();
            let unified = parts.next() == Some("1");
            Some(Gpu { name, memory_gb, discrete, unified, vulkan: Some((major.parse().ok()?, minor.parse().ok()?)) })
        })
        .collect()
}

#[allow(unsafe_code)] // Vulkan calls through `ash`: each is made with the handles this function creates and destroys.
fn probe(loader: &Path) -> Option<Vec<Gpu>> {
    use ash::vk;
    // SAFETY: loading the library runs its initialisers; that is why this runs in a child.
    let entry = unsafe { ash::Entry::load_from(loader) }.ok()?;
    // SAFETY: plain queries on a loaded entry.
    let version = unsafe { entry.try_enumerate_instance_version() }.ok()?.unwrap_or(vk::API_VERSION_1_0);
    let extensions = unsafe { entry.enumerate_instance_extension_properties(None) }.unwrap_or_default();
    let portability = extensions
        .iter()
        .any(|e| e.extension_name_as_c_str().is_ok_and(|n| n == ash::khr::portability_enumeration::NAME));
    let names = if portability { vec![ash::khr::portability_enumeration::NAME.as_ptr()] } else { Vec::new() };
    let app = vk::ApplicationInfo::default().api_version(version.min(vk::API_VERSION_1_3));
    let info = vk::InstanceCreateInfo::default()
        .application_info(&app)
        .enabled_extension_names(&names)
        // MoltenVK lists its devices only to an instance that asks for portability ones.
        .flags(if portability {
            vk::InstanceCreateFlags::ENUMERATE_PORTABILITY_KHR
        } else {
            vk::InstanceCreateFlags::empty()
        });
    // SAFETY: `info` and what it points to live until the call returns.
    let instance = unsafe { entry.create_instance(&info, None) }.ok()?;
    // SAFETY: the instance is alive until `destroy_instance` below, and every device comes from it.
    let gpus = unsafe { instance.enumerate_physical_devices() }
        .unwrap_or_default()
        .into_iter()
        .filter_map(|device| {
            let props = unsafe { instance.get_physical_device_properties(device) };
            if props.device_type == vk::PhysicalDeviceType::CPU {
                return None;
            }
            let discrete = props.device_type == vk::PhysicalDeviceType::DISCRETE_GPU;
            let memory = unsafe { instance.get_physical_device_memory_properties(device) };
            let local: u64 = memory
                .memory_heaps_as_slice()
                .iter()
                .filter(|h| h.flags.contains(vk::MemoryHeapFlags::DEVICE_LOCAL))
                .map(|h| h.size)
                .sum();
            Some(Gpu {
                name: props.device_name_as_c_str().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                // Built-in graphics share the processor's memory.
                memory_gb: discrete.then(|| local as f32 / (1u64 << 30) as f32),
                discrete,
                unified: !discrete && crate::hardware::APPLE_SILICON,
                vulkan: Some((vk::api_version_major(props.api_version), vk::api_version_minor(props.api_version))),
            })
        })
        .collect();
    unsafe { instance.destroy_instance(None) };
    Some(gpus)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_probe_s_lines_read_back() {
        let text = "gpu\tNVIDIA GeForce RTX 3090\t1.3\t1\t24.0\t0\ngpu\tApple M4 Max\t1.2\t0\t-\t1\nnoise\n";
        assert_eq!(
            parse(text),
            [
                Gpu {
                    name: "NVIDIA GeForce RTX 3090".into(),
                    memory_gb: Some(24.0),
                    discrete: true,
                    unified: false,
                    vulkan: Some((1, 3))
                },
                Gpu {
                    name: "Apple M4 Max".into(),
                    memory_gb: None,
                    discrete: false,
                    unified: true,
                    vulkan: Some((1, 2))
                },
            ]
        );
    }
}
