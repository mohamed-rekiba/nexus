//! Seam 1, core app API: how the core boots a set of plugins.

use nexus_api::PluginId;
use nexus_core::LoadError;
use nexus_testkit::{FakeHost, StubPlugin};

fn id(s: &str) -> PluginId {
    PluginId::new(s).unwrap()
}

fn emulator(id: &str, deps: &[&str]) -> StubPlugin {
    let deps: String = deps.iter().map(|d| format!("[[dependencies]]\nid = \"{d}\"\n")).collect();
    StubPlugin::from_toml(&format!(
        "id = \"{id}\"\nkind = \"emulator\"\nname = \"{id}\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\n{deps}"
    ))
}

#[tokio::test]
async fn a_dependency_cycle_rejects_every_plugin_in_it_and_nothing_else() {
    let host = FakeHost::new()
        .plugin(emulator("alpha", &["beta"]))
        .plugin(emulator("beta", &["alpha"]))
        .plugin(emulator("gamma", &[]))
        .boot()
        .await;

    let report = host.report();
    for member in ["alpha", "beta"] {
        assert_eq!(
            report.rejection(&id(member)),
            Some(&LoadError::Cycle { members: vec![id("alpha"), id("beta")] }),
            "{member} should be rejected as part of the cycle"
        );
    }
    assert_eq!(report.order, vec![id("gamma")]);
}

#[tokio::test]
async fn a_plugin_that_depends_on_a_rejected_plugin_is_rejected_too() {
    let host = FakeHost::new()
        .plugin(emulator("alpha", &["beta"]))
        .plugin(emulator("beta", &["alpha"]))
        .plugin(emulator("delta", &["alpha"]))
        .plugin(emulator("epsilon", &["missing"]))
        .boot()
        .await;

    let report = host.report();
    assert_eq!(report.rejection(&id("delta")), Some(&LoadError::DependencyRejected { dependency: id("alpha") }));
    assert_eq!(report.rejection(&id("epsilon")), Some(&LoadError::MissingDependency { dependency: id("missing") }));
    assert!(report.order.is_empty());
}

#[tokio::test]
async fn load_order_puts_dependencies_first_whatever_the_registration_order() {
    let host = FakeHost::new()
        .plugin(emulator("app", &["middle"]))
        .plugin(emulator("middle", &["base-lib"]))
        .plugin(emulator("base-lib", &[]))
        .boot()
        .await;

    assert_eq!(host.report().order, vec![id("base-lib"), id("middle"), id("app")]);
}

#[tokio::test]
async fn a_plugin_built_for_another_host_api_is_rejected_with_both_versions() {
    let plugin = StubPlugin::from_toml(
        "id = \"future\"\nkind = \"emulator\"\nname = \"Future\"\nversion = \"1.0.0\"\nhost_api = \"^2.0\"",
    );
    let host = FakeHost::new().plugin(plugin).boot().await;

    assert_eq!(
        host.report().rejection(&id("future")),
        Some(&LoadError::HostApi { required: "^2.0".into(), actual: nexus_api::API_VERSION.into() })
    );
}

#[tokio::test]
async fn an_emulator_for_another_platform_loads_turned_off_until_the_player_turns_it_on() {
    // A player may have their own build for this system, so it isn't rejected.
    let plugin = StubPlugin::from_toml(
        "id = \"elsewhere\"\nkind = \"emulator\"\nname = \"E\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\nplatforms = [\"windows\"]",
    );
    let t = FakeHost::new().platform(Some(nexus_api::Platform::Linux)).plugin(plugin).boot_core().await;
    assert!(t.report().rejection(&id("elsewhere")).is_none());
    let row = |t: &nexus_testkit::TestCore| {
        t.core.view().emulators.plugins.into_iter().find(|p| p.id == id("elsewhere")).unwrap()
    };
    assert!(!row(&t).enabled, "off until the player turns it on");
    assert!(row(&t).note.contains("Windows"), "{}", row(&t).note);

    t.core.dispatch(nexus_core::Command::SetPluginEnabled { plugin: id("elsewhere"), enabled: true }).await.unwrap();
    assert!(row(&t).enabled);
    t.core.dispatch(nexus_core::Command::SetPluginEnabled { plugin: id("elsewhere"), enabled: false }).await.unwrap();
    assert!(!row(&t).enabled);
}

#[tokio::test]
async fn a_theme_for_another_platform_is_rejected() {
    let plugin = StubPlugin::from_toml(
        "id = \"elsewhere\"\nkind = \"theme\"\nname = \"E\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\nplatforms = [\"windows\"]\ncapabilities = [\"theme\"]\n[runtime]\ntype = \"data\"",
    );
    let host = FakeHost::new().platform(Some(nexus_api::Platform::Linux)).plugin(plugin).boot().await;
    assert!(matches!(host.report().rejection(&id("elsewhere")), Some(LoadError::Platform { .. })));
}

#[tokio::test]
async fn a_dependency_with_the_wrong_version_is_rejected() {
    let lib = emulator("lib", &[]);
    let app = StubPlugin::from_toml(
        "id = \"app\"\nkind = \"emulator\"\nname = \"A\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\n\
         [[dependencies]]\nid = \"lib\"\nversion = \"^2\"",
    );
    let host = FakeHost::new().plugin(lib).plugin(app).boot().await;

    assert_eq!(
        host.report().rejection(&id("app")),
        Some(&LoadError::DependencyVersion { dependency: id("lib"), required: "^2".into(), found: "1.0.0".into() })
    );
}

#[tokio::test]
async fn a_missing_optional_dependency_is_ignored() {
    let app = StubPlugin::from_toml(
        "id = \"app\"\nkind = \"emulator\"\nname = \"A\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\n\
         [[dependencies]]\nid = \"extra\"\noptional = true",
    );
    let host = FakeHost::new().plugin(app).boot().await;

    assert_eq!(host.report().order, vec![id("app")]);
}

#[tokio::test]
async fn two_plugins_with_one_id_keep_the_first_and_reject_the_second() {
    let host = FakeHost::new().plugin(emulator("twin", &[])).plugin(emulator("twin", &[])).boot().await;

    assert_eq!(host.report().order, vec![id("twin")]);
    assert_eq!(host.report().rejection(&id("twin")), Some(&LoadError::Duplicate));
}

#[tokio::test]
async fn an_optional_dependency_in_the_wrong_version_is_ignored() {
    let lib = emulator("lib", &[]);
    let app = StubPlugin::from_toml(
        "id = \"app\"\nkind = \"emulator\"\nname = \"A\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\n\
         [[dependencies]]\nid = \"lib\"\nversion = \"^2\"\noptional = true",
    );
    let host = FakeHost::new().plugin(lib).plugin(app).boot().await;

    assert_eq!(host.report().rejection(&id("app")), None);
    assert!(host.report().order.contains(&id("app")));
}

#[tokio::test]
async fn a_folder_grant_names_a_host_folder_never_a_path() {
    let plugin = |id: &str, grant: &str| {
        StubPlugin::from_toml(&format!(
            "id = \"{id}\"\nkind = \"emulator\"\nname = \"R\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\n\
             [[permissions.filesystem]]\n{grant}"
        ))
    };
    let by_path = "id = \"p\"\nkind = \"emulator\"\nname = \"R\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\n\
                   [[permissions.filesystem]]\npath = \"/home\"\nwrite = true";
    let err = nexus_api::PluginManifest::from_toml(by_path).unwrap_err();
    assert!(err.to_string().contains("unknown field `path`"), "{err}");
    let host = FakeHost::new()
        .plugin(plugin("bad-name", "folder = \"/home\""))
        .plugin(plugin("named", "folder = \"pc-games\"\nwrite = true"))
        // The folders another file lists are read only.
        .plugin(plugin(
            "listed-writable",
            "folder = \"steam\"\nwrite = true\nlisted_in = { file = \"list.vdf\", key = \"path\", holds = \"steamapps\" }",
        ))
        .plugin(plugin("listed", "folder = \"steam\"\nlisted_in = { file = \"list.vdf\", key = \"path\", holds = \"steamapps\" }"))
        .boot()
        .await;

    let report = host.report();
    assert!(
        matches!(report.rejection(&id("bad-name")), Some(LoadError::Invalid(m)) if m.contains("folder name")),
        "{report:?}"
    );
    assert_eq!(report.rejection(&id("named")), None);
    assert!(
        matches!(report.rejection(&id("listed-writable")), Some(LoadError::Invalid(m)) if m.contains("can't be written")),
        "{report:?}"
    );
    assert_eq!(report.rejection(&id("listed")), None);
}

#[tokio::test]
async fn a_system_s_color_is_written_as_rrggbb() {
    let plugin = |id: &str, color: &str| {
        StubPlugin::from_toml(&format!(
            "id = \"{id}\"\nkind = \"emulator\"\nname = \"R\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\n\
             [system]\nid = \"R\"\nname = \"Retro\"\ncolor = \"{color}\""
        ))
    };
    let host = FakeHost::new().plugin(plugin("named-color", "red")).plugin(plugin("hex", "#ff8800")).boot().await;
    let report = host.report();
    assert!(
        matches!(report.rejection(&id("named-color")), Some(LoadError::Invalid(m)) if m.contains("#rrggbb")),
        "{report:?}"
    );
    assert_eq!(report.rejection(&id("hex")), None);
}

#[tokio::test]
async fn the_kernel_loads_an_emulator_and_a_theme_each_through_its_own_trait() {
    let emu = StubPlugin::from_toml(
        "id = \"emu\"\nkind = \"emulator\"\nname = \"E\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"",
    );
    let host =
        FakeHost::new().plugin(emu).built_in(nexus_api::PluginByKind::theme(theme_default::DefaultTheme)).boot().await;
    assert!(host.report().rejected.is_empty(), "{:?}", host.report().rejected);
    let order = host.kernel.order();
    assert!(order.contains(&id("emu")) && order.contains(&id("default")));
    let files = host.kernel.theme_files(&id("default")).expect("a theme has files");
    assert!(files.read(&nexus_api::RelPath::new("theme.toml").unwrap()).is_some());
    assert!(host.kernel.theme_files(&id("emu")).is_err(), "an emulator has no theme files");
}

#[tokio::test]
async fn code_of_one_kind_with_a_manifest_of_the_other_is_rejected() {
    // Emulator code whose manifest says it is a theme.
    let wrong = StubPlugin::from_toml(
        "id = \"wrong\"\nkind = \"theme\"\nname = \"W\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\ncapabilities = [\"theme\"]",
    );
    // Theme code whose manifest says it is an emulator.
    struct ThemeCode;
    impl nexus_api::Plugin for ThemeCode {
        fn manifest(&self) -> nexus_api::PluginManifest {
            nexus_api::PluginManifest::from_toml(
                "id = \"wrong-way\"\nkind = \"emulator\"\nname = \"W\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"",
            )
            .unwrap()
        }
    }
    impl nexus_api::ThemePlugin for ThemeCode {
        fn files(&self) -> std::sync::Arc<dyn nexus_api::Files> {
            <theme_default::DefaultTheme as nexus_api::ThemePlugin>::files(&theme_default::DefaultTheme)
        }
    }
    let host = FakeHost::new().plugin(wrong).built_in(nexus_api::PluginByKind::theme(ThemeCode)).boot().await;
    let rejection = |name: &str| match host.report().rejection(&id(name)) {
        Some(LoadError::Invalid(m)) => m.clone(),
        other => panic!("{name}: {other:?}"),
    };
    assert_eq!(rejection("wrong"), "the manifest says Theme, but the code is of kind Emulator");
    assert_eq!(rejection("wrong-way"), "the manifest says Emulator, but the code is of kind Theme");
}

#[tokio::test]
async fn a_service_loads_through_its_own_trait_and_may_not_run_games() {
    let service = |id: &str, capabilities: &str| {
        StubPlugin::from_toml(&format!(
            "id = \"{id}\"\nkind = \"service\"\nname = \"S\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\ncapabilities = [{capabilities}]"
        ))
    };
    let host = FakeHost::new()
        .plugin(service("store", "\"metadata\""))
        .plugin(service("runner", "\"metadata\", \"launcher\""))
        .boot()
        .await;
    assert_eq!(host.report().rejection(&id("store")), None);
    assert_eq!(host.kernel.of_kind(nexus_api::PluginKind::Service), [id("store")]);
    assert!(host.kernel.of_kind(nexus_api::PluginKind::Emulator).is_empty(), "a service isn't an emulator");
    assert!(
        matches!(host.report().rejection(&id("runner")), Some(LoadError::Invalid(m)) if m.contains("runs no games")),
        "{:?}",
        host.report().rejected
    );
}

/// The plugins and themes in plugins/ must accept the host's API and contract versions. Those are
/// set by hand, apart from Nexus's own version, which every release raises: when they were the
/// same, a release refused every built-in plugin.
#[test]
fn the_built_in_plugins_and_themes_accept_this_host() {
    let plugins = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugins");
    let manifests = [
        "emulators/kytyps5/plugin.toml",
        "emulators/shadps4/plugin.toml",
        "emulators/pc/plugin.toml",
        "services/release-catalog/plugin.toml",
        "services/playstation-store/plugin.toml",
        "system/host/plugin.toml",
        "themes/default/theme/theme.toml",
    ];
    for file in manifests {
        let text = std::fs::read_to_string(plugins.join(file)).unwrap();
        let plugin = nexus_api::PluginManifest::from_toml(&text).unwrap();
        assert!(
            plugin.host_api.matches(&nexus_api::api_version()),
            "{file}: host_api {} refuses API {}",
            plugin.host_api,
            nexus_api::API_VERSION
        );
        if file.ends_with("theme.toml") {
            let theme = nexus_ui_api::ThemeManifest::from_plugin(&plugin).unwrap();
            assert!(
                theme.theme.ui_api.matches(&nexus_ui_api::ui_api_version()),
                "{file}: ui_api {} refuses contract {}",
                theme.theme.ui_api,
                nexus_ui_api::UI_API_VERSION
            );
        }
    }
}
