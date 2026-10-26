//! What a PC game's files are, and what starts the game on each system.
//!
//! A release comes as a folder of files, often with the program flag lost on the way, so files
//! are known by their first bytes, not their names: a start script, an AppImage, a Linux (ELF)
//! program, a Mac (Mach-O) program, or a Windows (PE) program.

use nexus_api::Platform;
use nexus_api::serde::{Deserialize, Serialize};

/// How many bytes [`probe`] reads: they hold a program's headers.
pub const HEAD: u64 = 4096;

/// What a file is, from its first bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Probe {
    /// A Linux program (an executable, or a library with an interpreter): x86-64 or not.
    Elf {
        x64: bool,
    },
    AppImage,
    /// Starts with `#!`.
    Script,
    /// A Windows program.
    Exe,
    /// A Mac program.
    MachO,
    Other,
}

impl Probe {
    /// Whether it is a program: a Linux, Windows or Mac one, or an AppImage.
    pub fn is_program(self) -> bool {
        matches!(self, Probe::Elf { .. } | Probe::Exe | Probe::AppImage | Probe::MachO)
    }

    /// Whether it should have the program flag (downloads drop it).
    pub fn runnable(self) -> bool {
        matches!(self, Probe::Elf { .. } | Probe::AppImage | Probe::Script | Probe::MachO)
    }
}

/// What a file is, from its first [`HEAD`] bytes.
pub fn probe(head: &[u8]) -> Probe {
    if head.starts_with(b"#!") {
        return Probe::Script;
    }
    // Mach-O, 64-bit and 32-bit, either byte order, and a universal ("fat") binary.
    const MACHO: [[u8; 4]; 5] = [
        [0xcf, 0xfa, 0xed, 0xfe],
        [0xce, 0xfa, 0xed, 0xfe],
        [0xfe, 0xed, 0xfa, 0xcf],
        [0xfe, 0xed, 0xfa, 0xce],
        [0xca, 0xfe, 0xba, 0xbe],
    ];
    if head.len() >= 8 && MACHO.iter().any(|m| head.starts_with(m)) {
        // `cafebabe` is also a Java class: a fat binary has few architectures, a class file a
        // version above 40.
        let fat = head.starts_with(&[0xca, 0xfe, 0xba, 0xbe]);
        let count = u32::from_be_bytes([head[4], head[5], head[6], head[7]]);
        return if !fat || count < 20 { Probe::MachO } else { Probe::Other };
    }
    if head.starts_with(b"MZ") && head.len() >= 0x40 {
        let pe = u32::from_le_bytes([head[0x3c], head[0x3d], head[0x3e], head[0x3f]]) as usize;
        return if head.get(pe..pe + 4) == Some(b"PE\0\0".as_slice()) { Probe::Exe } else { Probe::Other };
    }
    if !head.starts_with(b"\x7fELF") || head.len() < 0x34 {
        return Probe::Other;
    }
    if head.get(8..11) == Some(b"AI\x02".as_slice()) {
        return Probe::AppImage;
    }
    let is64 = head[4] == 2;
    if head[5] != 1 {
        return Probe::Other; // big endian: not for this computer
    }
    let u16_at = |o: usize| head.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize);
    let u32_at = |o: usize| head.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    let u64_at = |o: usize| {
        head.get(o..o + 8).map(|b| u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]) as usize)
    };
    let (Some(kind), Some(machine)) = (u16_at(16), u16_at(18)) else { return Probe::Other };
    let x64 = machine == 0x3e;
    if !(x64 || machine == 0x03) {
        return Probe::Other; // ARM and others
    }
    // ET_EXEC is a program; ET_DYN is a program only with an interpreter (PIE), else a library.
    if kind == 2 {
        return Probe::Elf { x64 };
    }
    if kind != 3 {
        return Probe::Other;
    }
    let (phoff, size, count) =
        if is64 { (u64_at(0x20), u16_at(0x36), u16_at(0x38)) } else { (u32_at(0x1c), u16_at(0x2a), u16_at(0x2c)) };
    let (Some(phoff), Some(size), Some(count)) = (phoff, size, count) else { return Probe::Other };
    for i in 0..count.min(64) {
        match u32_at(phoff + i * size) {
            Some(3) => return Probe::Elf { x64 }, // PT_INTERP
            Some(_) => {}
            None => break,
        }
    }
    Probe::Other
}

/// How an installed game starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(crate = "nexus_api::serde", rename_all = "lowercase")]
pub enum Kind {
    /// A shell script: the release's `start.sh`, a GOG `start.sh`, a repack's `start`.
    Script,
    AppImage,
    /// A Linux program.
    Native,
    /// A Mac program, or the program inside a `.app`.
    MacApp,
    /// A Windows program: run as it is on Windows, through Proton or Wine elsewhere.
    Windows,
}

impl Kind {
    /// Whether this system can start it (a Windows game with a runner; see [`Kind::needs_runner`]).
    pub fn runs_on(self, platform: Platform) -> bool {
        match self {
            Kind::Script | Kind::AppImage | Kind::Native => platform == Platform::Linux,
            Kind::MacApp => platform == Platform::Macos,
            Kind::Windows => true,
        }
    }

    /// Whether it needs Proton or Wine on this system.
    pub fn needs_runner(self, platform: Platform) -> bool {
        self == Kind::Windows && platform != Platform::Windows
    }

    /// Why this system can't start it.
    pub fn not_here(self, platform: Platform) -> String {
        let what = match self {
            Kind::Script | Kind::AppImage | Kind::Native => "a Linux game",
            Kind::MacApp => "a Mac game",
            Kind::Windows => "a Windows game",
        };
        format!("This is {what}: it can't run on {}", system_name(platform))
    }
}

pub fn system_name(platform: Platform) -> &'static str {
    match platform {
        Platform::Linux => "Linux",
        Platform::Macos => "a Mac",
        Platform::Windows => "Windows",
    }
}

/// Helpers that come with games but don't start them: listed last.
fn helper(name: &str) -> bool {
    const WORDS: [&str; 22] = [
        "setup",
        "unins",
        "install",
        "redist",
        "vcredist",
        "dxsetup",
        "directx",
        "dotnet",
        "physx",
        "crashhandler",
        "crash_handler",
        "crashpad",
        "crashreport",
        "bugreport",
        "uploader",
        "desktop",
        "settings",
        "config",
        "uninstall",
        "update",
        "steam_api",
        "easyanticheat",
    ];
    let n = name.to_ascii_lowercase();
    WORDS.iter().any(|w| n.contains(w))
}

/// A library, which no one starts: not listed.
fn library(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.ends_with(".so") || n.contains(".so.") || n.ends_with(".dll")
}

/// Folders of runtimes a game brings (Wine, Proton, Steam's runtime, Java, libraries): their
/// programs aren't listed.
const RUNTIMES: [&str; 12] = [
    "lib/",
    "lib32/",
    "lib64/",
    "libs/",
    "/wine/",
    "wine/bin",
    "files/bin",
    "runtime/",
    "dist/",
    "mono/",
    "jre/",
    "java/",
];

/// The names a start script goes by.
const START_NAMES: [&str; 8] =
    ["start", "start.sh", "start-game.sh", "start_game.sh", "play.sh", "run.sh", "launch.sh", "game.sh"];

/// Whether the file at `path` is named as a start script is (`start.sh`, `run.sh`, …).
pub fn start_script(path: &str) -> bool {
    START_NAMES.contains(&path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase().as_str())
}

/// Each of `paths` as people read it: its file name, and when another has the same file name,
/// the nearest folder that tells them apart (`ENG/start.sh` beside `RUS/start.sh`,
/// `ENG/…/Ra2.exe` beside `RUS/…/Ra2.exe`).
pub fn short_names(paths: &[String]) -> Vec<String> {
    let parts = |p: &str| p.rsplit('/').map(str::to_owned).collect::<Vec<_>>();
    paths
        .iter()
        .map(|path| {
            let mine = parts(path);
            let twins: Vec<Vec<String>> =
                paths.iter().filter(|o| *o != path).map(|o| parts(o)).filter(|o| o[0] == mine[0]).collect();
            if twins.is_empty() {
                return mine[0].clone();
            }
            match (1..mine.len()).find(|&i| twins.iter().all(|t| t.get(i) != Some(&mine[i]))) {
                Some(1) => format!("{}/{}", mine[1], mine[0]),
                Some(i) => format!("{}/…/{}", mine[i], mine[0]),
                None => path.clone(),
            }
        })
        .collect()
}

/// Every file of a game that can start it, from its files (paths from its folder) and what
/// each is, best first. Helpers (setups, crash reporters) come last. Then by what the file
/// is: a start script by its name, an AppImage, a Mac program in a `.app`, a Linux program,
/// another Mac program, another script, a Windows program. Then nearer the game's folder
/// first, a 64-bit program before a 32-bit one, and by path. Libraries and the programs of
/// bundled runtimes (see [`RUNTIMES`]) aren't listed. Empty when nothing in it starts a game.
pub fn rank(files: &[(String, Probe)]) -> Vec<(String, Kind)> {
    /// A file's place in the list, compared field by field.
    #[derive(PartialEq, Eq, PartialOrd, Ord)]
    struct Place<'a> {
        helper: bool,
        order: u8,
        depth: usize,
        not_x64: bool,
        path: &'a str,
    }
    let mut ranked: Vec<(Place, Kind)> = Vec::new();
    for (path, probe) in files {
        let file = path.rsplit('/').next().unwrap_or(path);
        let lower = file.to_ascii_lowercase();
        let dir_lower = path.to_ascii_lowercase();
        if library(file) || (RUNTIMES.iter().any(|d| dir_lower.contains(d)) && !start_script(file)) {
            continue;
        }
        let (order, kind, x64) = match probe {
            Probe::Script | Probe::Other if START_NAMES.contains(&lower.as_str()) => (0, Kind::Script, true),
            Probe::AppImage => (1, Kind::AppImage, true),
            Probe::MachO if dir_lower.contains(".app/contents/macos/") => (2, Kind::MacApp, true),
            Probe::Elf { x64 } => (3, Kind::Native, *x64),
            Probe::MachO => (4, Kind::MacApp, true),
            Probe::Script => (5, Kind::Script, true),
            Probe::Exe => (6, Kind::Windows, true),
            Probe::Other => continue,
        };
        let place = Place { helper: helper(file), order, depth: path.matches('/').count(), not_x64: !x64, path };
        ranked.push((place, kind));
    }
    ranked.sort_by(|a, b| a.0.cmp(&b.0));
    ranked.into_iter().map(|(place, kind)| (place.path.to_owned(), kind)).collect()
}

/// A game's name from its release folder: "[GOG] Celeste (v1.4.0.0) [amd64] [Multi]" is
/// "Celeste". Leading tags, tags after the name, a bracketed version, "+ DLC" and a bare
/// version at the end go.
pub fn clean_name(title: &str) -> String {
    let mut name = title.trim();
    while let Some(rest) = name.strip_prefix('[').and_then(|r| r.split_once(']')).map(|(_, rest)| rest.trim_start()) {
        name = rest;
    }
    let name = name.split(" [").next().unwrap_or(name).replace('_', " ");
    // A bracketed group that is only a version or a build: "(v1.4.0.0)", "(build 9218157)".
    let mut out = String::new();
    let mut rest = name.as_str();
    while let Some(open) = rest.find('(') {
        let Some(close) = rest[open..].find(')').map(|c| open + c) else { break };
        let group = &rest[open + 1..close];
        let lower = group.to_ascii_lowercase();
        let versionish = lower.starts_with('v')
            && lower[1..].trim_start_matches('.').starts_with(|c: char| c.is_ascii_digit())
            || lower.starts_with("build")
            || lower.starts_with('+')
            || lower.starts_with(|c: char| c.is_ascii_digit()) && lower.chars().all(|c| c.is_ascii_digit() || c == '.');
        out.push_str(&rest[..open]);
        if !versionish {
            out.push_str(&rest[open..=close]);
        }
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    let mut name = out.split_whitespace().collect::<Vec<_>>().join(" ");
    // "+ 3 DLC", "+ OST" at the end.
    if let Some(at) = name.find(" + ") {
        name.truncate(at);
    }
    // A bare version at the end: "Game v3.3.1".
    if let Some((head, last)) = name.rsplit_once(' ')
        && last
            .to_ascii_lowercase()
            .strip_prefix('v')
            .is_some_and(|v| v.trim_start_matches('.').starts_with(|c: char| c.is_ascii_digit()))
    {
        name = head.to_owned();
    }
    let name = name.trim().to_owned();
    if name.is_empty() { title.trim().to_owned() } else { name }
}

/// A game's name from a folder's or a file's: [`clean_name`], and a name in small letters
/// ("cuphead_v1.3.4.run", "my_game") with a capital at each word.
pub fn display_name(title: &str) -> String {
    let name = clean_name(title);
    if name.chars().any(char::is_uppercase) {
        return name;
    }
    name.split(' ')
        .map(|word| {
            let mut letters = word.chars();
            letters.next().map(|first| first.to_uppercase().chain(letters).collect()).unwrap_or_default()
        })
        .collect::<Vec<String>>()
        .join(" ")
}

/// A folder name for a game: its name in safe characters.
pub fn folder_name(name: &str) -> String {
    let s: String =
        name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '-' }).collect();
    let s = s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    let s: String = s.trim_matches(|c| c == '.' || c == '-').chars().take(60).collect();
    if s.is_empty() { "game".into() } else { s }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The first bytes of an ELF file: `kind` 2 (a program) or 3 (with an interpreter when
    /// `interp`), for `machine`.
    pub fn elf(kind: u16, machine: u16, interp: bool) -> Vec<u8> {
        let mut h = vec![0u8; 0x200];
        h[..4].copy_from_slice(b"\x7fELF");
        h[4] = 2;
        h[5] = 1;
        h[16..18].copy_from_slice(&kind.to_le_bytes());
        h[18..20].copy_from_slice(&machine.to_le_bytes());
        h[0x20..0x28].copy_from_slice(&0x40u64.to_le_bytes());
        h[0x36..0x38].copy_from_slice(&56u16.to_le_bytes());
        h[0x38..0x3a].copy_from_slice(&2u16.to_le_bytes());
        h[0x40..0x44].copy_from_slice(&6u32.to_le_bytes()); // PT_PHDR
        h[0x40 + 56..0x40 + 60].copy_from_slice(&(if interp { 3u32 } else { 1 }).to_le_bytes());
        h
    }

    pub fn exe() -> Vec<u8> {
        let mut pe = vec![0u8; 0x100];
        pe[..2].copy_from_slice(b"MZ");
        pe[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        pe[0x80..0x84].copy_from_slice(b"PE\0\0");
        pe
    }

    #[test]
    fn files_are_known_by_their_first_bytes() {
        assert_eq!(probe(&elf(3, 0x3e, true)), Probe::Elf { x64: true }, "a PIE program");
        assert_eq!(probe(&elf(3, 0x3e, false)), Probe::Other, "a library");
        assert_eq!(probe(&elf(2, 0x03, false)), Probe::Elf { x64: false }, "a 32-bit program");
        assert_eq!(probe(&elf(2, 0xb7, false)), Probe::Other, "an ARM program");
        let mut appimage = elf(2, 0x3e, true);
        appimage[8..11].copy_from_slice(b"AI\x02");
        assert_eq!(probe(&appimage), Probe::AppImage);
        assert_eq!(probe(b"#!/bin/bash\n./game"), Probe::Script);
        assert_eq!(probe(&exe()), Probe::Exe);
        assert_eq!(probe(&[0xcf, 0xfa, 0xed, 0xfe, 7, 0, 0, 1]), Probe::MachO);
        assert_eq!(probe(&[0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 2]), Probe::MachO, "a universal binary");
        assert_eq!(probe(&[0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 52]), Probe::Other, "a Java class");
        assert_eq!(probe(b"PNG data"), Probe::Other);
    }

    #[test]
    fn every_file_that_starts_the_game_is_listed_the_start_script_first() {
        let files = vec![
            ("start.sh".to_owned(), Probe::Script),
            ("game/Celeste.x86_64".to_owned(), Probe::Elf { x64: true }),
            ("game/Celeste.exe".to_owned(), Probe::Exe),
            ("game/lib64/libfmod.so".to_owned(), Probe::Elf { x64: true }),
            ("UnityCrashHandler64.exe".to_owned(), Probe::Exe),
            ("Celeste.app/Contents/MacOS/Celeste".to_owned(), Probe::MachO),
            ("readme.txt".to_owned(), Probe::Other),
        ];
        let paths: Vec<_> = rank(&files).into_iter().map(|(p, _)| p).collect();
        assert_eq!(
            paths,
            [
                "start.sh",
                "Celeste.app/Contents/MacOS/Celeste",
                "game/Celeste.x86_64",
                "game/Celeste.exe",
                "UnityCrashHandler64.exe"
            ],
            "the helper last; no library or text file"
        );
        assert!(rank(&[("readme.txt".to_owned(), Probe::Other)]).is_empty());
    }

    #[test]
    fn the_order_is_the_file_s_type_then_how_deep_it_is_whatever_its_name() {
        let files = vec![
            ("Game.AppImage".to_owned(), Probe::AppImage),
            ("bin/x86/game".to_owned(), Probe::Elf { x64: false }),
            ("bin/x86_64/game".to_owned(), Probe::Elf { x64: true }),
            ("Desktop Dungeons/desktopdungeons".to_owned(), Probe::Elf { x64: true }),
            ("tools/build.sh".to_owned(), Probe::Script),
            ("a/b/c/start.sh".to_owned(), Probe::Script),
            ("wine/bin/wine".to_owned(), Probe::Elf { x64: false }),
        ];
        let paths: Vec<_> = rank(&files).into_iter().map(|(p, _)| p).collect();
        assert_eq!(
            paths,
            [
                "a/b/c/start.sh",
                "Game.AppImage",
                "bin/x86_64/game",
                "bin/x86/game",
                "tools/build.sh",
                "Desktop Dungeons/desktopdungeons"
            ],
            "a deep start script first; 64-bit before 32-bit; a name with a helper's word last; not its Wine"
        );
        assert_eq!(Kind::Native.not_here(Platform::Windows), "This is a Linux game: it can't run on Windows");
    }

    #[test]
    fn a_release_folder_s_name_is_the_game_s_name() {
        assert_eq!(clean_name("[GOG] Celeste (v1.4.0.0) [amd64] [Multi] [Native]"), "Celeste");
        assert_eq!(clean_name("Hollow Knight (build 1.5.78) + 3 DLC [Wine]"), "Hollow Knight");
        assert_eq!(clean_name("DROD The City Beneath v3.3.1"), "DROD The City Beneath");
        assert_eq!(clean_name("Age of Empires II (Definitive Edition)"), "Age of Empires II (Definitive Edition)");
        assert_eq!(clean_name("Some_Game_2024"), "Some Game 2024");
        assert_eq!(folder_name("Hollow Knight: Silksong"), "Hollow-Knight-Silksong");
        assert_eq!(folder_name("::"), "game");
    }

    #[test]
    fn a_start_is_named_by_its_file_with_the_folders_that_tell_it_apart() {
        let paths = [
            "ENG/start.sh",
            "RUS/start.sh",
            "ENG/data/gamemd.exe",
            "bin/x86/game",
            "bin/x86_64/game",
            "ENG/game_info/data/Ra2.exe",
            "RUS/game_info/data/Ra2.exe",
        ];
        let paths: Vec<String> = paths.iter().map(|p| p.to_string()).collect();
        assert_eq!(
            short_names(&paths),
            ["ENG/start.sh", "RUS/start.sh", "gamemd.exe", "x86/game", "x86_64/game", "ENG/…/Ra2.exe", "RUS/…/Ra2.exe"]
        );
        assert!(start_script("data/noarch/start.sh") && start_script("Run.sh") && !start_script("game.exe"));
    }
}
