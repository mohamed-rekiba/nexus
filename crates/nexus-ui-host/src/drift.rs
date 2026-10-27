//! The drift test: the view models and the theme contract name the same fields.
//!
//! `types.slint` is written by hand, for theme authors to read. This test reads its structs
//! and its `Vm` properties, converts a view with every part filled, and fails on a field one
//! side has and the other doesn't. It also sets every value on the compiled contract, so a
//! field of the wrong type fails too.

use std::collections::{BTreeMap, BTreeSet};

use nexus_api::{ActionStyle, ConfigScope, GameFact, KeyBinding, PluginId, Rating, SlotId, Tone};
use nexus_ui_api::*;
use slint::Model;
use slint_interpreter::Value;

use crate::bind::{self, CardModels, ImageSource};
use crate::{Crop, ImageSize};

/// The `Vm` properties that change on events, set one by one, not with the view.
const SET_ON_EVENTS: [&str; 4] = ["art-ready", "trailer-frame", "pad-key", "power-menu"];

/// The contract as the test reads it: each struct's fields, and each `Vm` property, with their
/// types as written.
struct Contract {
    structs: BTreeMap<String, BTreeMap<String, String>>,
    vm: BTreeMap<String, String>,
}

/// `source` without its `//` comments. The contract has no `/* */` ones.
fn code(source: &str) -> String {
    source.lines().map(|l| l.split("//").next().unwrap_or_default()).collect::<Vec<_>>().join("\n")
}

fn read_contract() -> Contract {
    let code = code(contract::TYPES_SLINT);
    let mut structs = BTreeMap::new();
    for block in code.split("export struct ").skip(1) {
        let (name, rest) = block.split_once('{').expect("a struct has a body");
        let body = rest.split_once('}').expect("a struct ends").0;
        let fields = body
            .split(',')
            .filter_map(|f| f.split_once(':'))
            .map(|(name, ty)| (name.trim().to_owned(), ty.trim().to_owned()))
            .collect();
        structs.insert(name.trim().to_owned(), fields);
    }
    let global = code.split("export global Vm {").nth(1).expect("the Vm global").split("\n}").next().unwrap();
    let vm = global
        .split(';')
        .filter_map(|p| p.split_once("property <"))
        .map(|(_, rest)| {
            let (ty, name) = rest.split_once('>').expect("a property's type");
            let name = name.split(':').next().unwrap_or_default();
            (name.trim().to_owned(), ty.trim().to_owned())
        })
        .collect();
    Contract { structs, vm }
}

impl Contract {
    /// Where `value`, of the contract type `ty` at `path`, has other fields than the contract.
    fn differences(&self, path: &str, ty: &str, value: &Value, out: &mut Vec<String>) {
        if let Some(row) = ty.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
            let Value::Model(rows) = value else { return out.push(format!("{path}: not a list")) };
            match rows.row_data(0) {
                Some(first) => self.differences(&format!("{path}[0]"), row, &first, out),
                None if self.structs.contains_key(row) => out.push(format!("{path}: empty in the test's view")),
                None => {}
            }
            return;
        }
        let Some(declared) = self.structs.get(ty) else { return };
        let Value::Struct(fields) = value else { return out.push(format!("{path}: not a struct")) };
        let given: BTreeSet<&str> = fields.iter().map(|(name, _)| name).collect();
        for name in declared.keys().filter(|n| !given.contains(n.as_str())) {
            out.push(format!("{path}.{name}: in types.slint, not in the view"));
        }
        for (name, value) in fields.iter() {
            match declared.get(name) {
                Some(ty) => self.differences(&format!("{path}.{name}"), ty, value, out),
                None => out.push(format!("{path}.{name}: in the view, not in types.slint")),
            }
        }
    }
}

/// Images that are never in.
struct NoImages;

impl ImageSource for NoImages {
    fn image(&self, _: &ImageRef, _: ImageSize, _: Crop, _: bool) -> slint_interpreter::Image {
        slint_interpreter::Image::default()
    }
    fn failed(&self, _: &ImageRef, _: ImageSize, _: Crop) -> bool {
        false
    }
    fn near(&self, _: &str, _: usize) -> bool {
        true
    }
}

fn id() -> PluginId {
    PluginId::new("emu").unwrap()
}

fn s() -> String {
    "x".into()
}

fn image() -> ImageRef {
    ImageRef::Url { plugin: id(), url: "https://art.example/a.png".into() }
}

fn slot() -> SlotVm {
    SlotVm {
        slot: SlotId::GameBadge,
        plugin: id(),
        id: s(),
        label: s(),
        kind: SlotKind::Badge,
        text: s(),
        short: s(),
        tone: Tone::Info,
        style: ActionStyle::Primary,
        icon: Some(image()),
        rows: vec![RowVm { label: s(), value: s() }],
        action: Some(s()),
        args: serde_json::json!({ "a": 1 }),
        scope: Some(ConfigScope::Emulator),
    }
}

fn card() -> GameCardVm {
    GameCardVm {
        key: "emu:a".into(),
        title: s(),
        platform: "PS4".into(),
        system: GameSystemVm { name: s(), color: Some([1, 2, 3]) },
        cover: Some(image()),
        selected: true,
        badges: vec![slot()],
        release_year: Some(2024),
        rating: Some(4.0),
        size_bytes: Some(1),
        size: s(),
        last_played_days: Some(1),
        last_played: s(),
        releases: 1,
        availability: s(),
        availability_text: s(),
        hidden: true,
        can: GameActionsVm::default(),
    }
}

fn filter() -> FilterVm {
    FilterVm { id: s(), label: s(), count: 1, on: true }
}

fn library() -> LibraryVm {
    LibraryVm {
        games: vec![card()],
        total: 1,
        all: 1,
        selected: Some(0),
        systems: vec![filter()],
        genres: vec![filter()],
        sort: s(),
        query: s(),
    }
}

fn field() -> FieldVm {
    FieldVm {
        key: s(),
        label: s(),
        help: s(),
        kind: FieldKind::Choice,
        value: s(),
        choices: vec![s()],
        choice_labels: vec![s()],
    }
}

fn panel() -> SettingsPanelVm {
    SettingsPanelVm { slot: slot(), fields: vec![field()] }
}

fn verdict() -> VerdictVm {
    VerdictVm { plugin: s(), emulator: s(), level: s(), headline: s(), why: s() }
}

fn folder() -> FolderVm {
    FolderVm {
        name: s(),
        title: s(),
        help: s(),
        paths: vec![s()],
        text: s(),
        shown: s(),
        default: s(),
        requested_by: vec![s()],
        requested_by_text: s(),
        write: true,
    }
}

fn emulator() -> EmulatorVm {
    EmulatorVm {
        plugin: id(),
        name: s(),
        status: s(),
        has_builds: true,
        installed: vec![s()],
        available: vec![s()],
        update: Some(s()),
        busy: true,
        panels: vec![slot()],
        controllers: vec![s()],
        controllers_text: s(),
    }
}

fn plugin_row() -> PluginRowVm {
    PluginRowVm {
        id: id(),
        name: s(),
        kind: s(),
        version: s(),
        status: s(),
        enabled: true,
        note: s(),
        build: s(),
        build_label: s(),
        has_builds: true,
    }
}

fn detail() -> GameDetailVm {
    GameDetailVm {
        key: "emu:a".into(),
        title: s(),
        platform: "PS4".into(),
        system: GameSystemVm { name: s(), color: None },
        description: s(),
        summary: s(),
        developer: s(),
        genres: vec![s()],
        cover: Some(image()),
        background: Some(image()),
        compat: Some(CompatVm { level: s(), label: s(), notes: s(), tested: s(), tested_text: s() }),
        badges: vec![slot()],
        actions: vec![slot()],
        sections: vec![slot()],
        can: GameActionsVm::default(),
        settings: vec![panel()],
        launch_hint: s(),
        running: true,
        size_bytes: Some(1),
        size: s(),
        logo: Some(image()),
        rating: Some(Rating { score: 4.0, total: 2 }),
        rating_score: 4.0,
        ratings: 2,
        rating_text: s(),
        release: s(),
        release_text: s(),
        year: s(),
        age_rating: s(),
        screenshots: vec![image()],
        store_url: s(),
        protondb: s(),
        protondb_url: s(),
        this_pc: Some(verdict()),
        pc_warning: s(),
        your_result: s(),
        has_folder: true,
        has_log: true,
        hidden: true,
        compat_page: s(),
        trailer_url: s(),
        region: s(),
        age: s(),
        version: s(),
        facts: vec![GameFact::new("region", "Region", "Europe")],
        played_secs: 1,
        played: s(),
        last_played_days: Some(1),
        last_played: s(),
        releases: vec![ReleaseVm {
            id: s(),
            store: s(),
            version: s(),
            region: s(),
            size_bytes: Some(1),
            size: s(),
            added: s(),
            added_text: s(),
            new: true,
            description: s(),
            system: s(),
            not_here: s(),
            warning: s(),
            no_room: s(),
            download: s(),
            download_progress: 0.5,
            version_label: s(),
            best: true,
            seeders: Some(1),
            leechers: Some(1),
            counted_at: s(),
            counted_text: s(),
            availability: s(),
            availability_text: s(),
        }],
        emulator: s(),
        download_folder: s(),
        availability: s(),
        availability_text: s(),
        download_error: s(),
    }
}

/// A view with every part there: each `Option` is `Some`, each list has a row.
fn full_view() -> AppView {
    AppView {
        screen: ScreenId::Home,
        game: Some(s()),
        can_go_back: true,
        home: HomeVm {
            games: vec![card()],
            selected: Some(0),
            loading: true,
            groups: vec![GroupVm { platform: s(), games: vec![card()], selected: None }],
            library: library(),
        },
        detail: Some(detail()),
        catalog: CatalogVm {
            available: true,
            games: library(),
            sources: vec![s()],
            releases: 1,
            updated: s(),
            updated_text: s(),
            loading: true,
            actions: vec![slot()],
            status: vec![slot()],
        },
        downloads: DownloadsVm {
            jobs: vec![DownloadVm {
                id: 1,
                plugin: id(),
                label: s(),
                received: 1,
                total: Some(2),
                progress: 0.5,
                progress_text: s(),
                state: s(),
                error: s(),
                can_pause: true,
                can_resume: true,
                can_cancel: true,
            }],
            torrents: vec![TorrentVm { files: vec![s()], actions: vec![s()], ..TorrentVm::default() }],
            seeding: true,
            downloading: 1,
            sharing: 1,
        },
        settings: SettingsVm {
            launcher_update: LauncherUpdateVm::default(),
            hardware: HardwareVm { graphics: vec![s()], verdicts: vec![verdict()], ..HardwareVm::default() },
            panels: vec![panel()],
            themes: vec![ThemeChoiceVm { id: id(), name: s(), current: true }],
            theme_options: vec![field()],
            folders: vec![folder()],
            vpn: VpnSettingsVm { vpns: vec![VpnVm::default()], ..VpnSettingsVm::default() },
            ..SettingsVm::default()
        },
        emulators: EmulatorsVm {
            emulators: vec![emulator()],
            plugins: vec![plugin_row()],
            emulator_plugins: vec![plugin_row()],
            theme_plugins: vec![plugin_row()],
        },
        plugin_page: Some(PluginPageVm {
            plugin: plugin_row(),
            emulator: Some(emulator()),
            panels: vec![panel()],
            sections: vec![slot()],
            actions: vec![slot()],
            folders: vec![folder()],
            current: true,
        }),
        running: Some(RunningVm {
            session: 1,
            key: s(),
            title: s(),
            emulator: s(),
            page: s(),
            state: s(),
            elapsed_secs: 1,
            elapsed: s(),
            in_front: true,
            keymap: vec![KeyBinding { button: s(), keys: s() }],
        }),
        notices: vec![s()],
        notice_cards: vec![NoticeVm { id: 1, text: s(), kind: NoticeKind::Info, art: Some(image()), opens: s() }],
        install_prompt: Some(InstallPromptVm {
            plugin: id(),
            emulator: s(),
            version: s(),
            game: s(),
            size_bytes: Some(1),
        }),
        boot: BootVm {
            steps: vec![BootStepVm { label: s(), detail: s(), state: s() }],
            covers: vec![image()],
            ..BootVm::default()
        },
        task: Some(s()),
        notice_art: Some(image()),
        notice_opens: s(),
        result_prompt: Some(s()),
        system: SystemVm {
            wifi: vec![WifiVm { ssid: s(), bars: 1, secured: true, saved: true, active: true }],
            outputs: vec![AudioOutputVm { name: s(), description: s(), current: true }],
            devices: vec![BluetoothDeviceVm { address: s(), name: s(), paired: true, connected: true }],
            disks: vec![InstallDiskVm { path: s(), size: s(), windows: true, usb: true, model: s() }],
            jobs: vec![SystemJobVm {
                id: 1,
                kind: s(),
                target: s(),
                state: s(),
                percent: 1,
                text: s(),
                mok: true,
                nvidia_later: true,
            }],
            ..SystemVm::default()
        },
    }
}

fn host_view(monitors: &[String]) -> bind::HostVm<'_> {
    bind::HostVm {
        viewport: bind::Viewport { width: 1.0, height: 1.0, scale: 1.0, size: 1.0, aspect: 1.0, breakpoint: s() },
        input_mode: "pointer",
        clock: "9:38 PM",
        can_play_trailers: true,
        pad_name: "pad",
        monitors,
        monitor: s(),
        has_trailer: true,
        trailer: bind::TrailerVm::default(),
    }
}

/// Every `Vm` property of a full view, and of the host.
fn all_values() -> Vec<(String, Value)> {
    let view = full_view();
    let monitors = vec![s()];
    let mut values = bind::vm_values(&view, &NoImages, &CardModels::default()).unwrap();
    values.extend(bind::host_values(&host_view(&monitors), &NoImages).unwrap());
    values
}

#[test]
fn the_view_models_and_types_slint_name_the_same_fields() {
    let contract = read_contract();
    assert!(contract.structs.contains_key("GameCard") && contract.vm.contains_key("home"), "the reader reads");
    let values = all_values();
    let mut differences = Vec::new();
    let given: BTreeSet<&str> = values.iter().map(|(name, _)| name.as_str()).collect();
    for name in contract.vm.keys().filter(|n| !given.contains(n.as_str()) && !SET_ON_EVENTS.contains(&n.as_str())) {
        differences.push(format!("Vm.{name}: in types.slint, not in the view"));
    }
    for (name, value) in &values {
        match contract.vm.get(name) {
            Some(ty) => contract.differences(&format!("Vm.{name}"), ty, value, &mut differences),
            None => differences.push(format!("Vm.{name}: in the view, not in types.slint")),
        }
    }
    assert!(differences.is_empty(), "the contract drifted:\n{}", differences.join("\n"));
}

#[test]
fn every_value_of_the_view_has_its_contract_type() {
    i_slint_backend_testing::init_no_event_loop();
    let source = format!("{}\nexport component Shell inherits Window {{}}\n", contract::TYPES_SLINT);
    let compiler = slint_interpreter::Compiler::default();
    let result = spin_on::spin_on(compiler.build_from_source(source, "types.slint".into()));
    let errors: Vec<String> = result.diagnostics().map(|d| d.to_string()).collect();
    let instance = result.component("Shell").unwrap_or_else(|| panic!("{errors:?}")).create().unwrap();
    for (name, value) in all_values() {
        instance.set_global_property("Vm", &name, value).unwrap_or_else(|e| panic!("Vm.{name}: {e}"));
    }
}
