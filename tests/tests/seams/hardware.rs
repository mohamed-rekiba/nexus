//! Seam 1, core app API: "This PC". Each emulator's manifest says what its games need; the
//! core compares it with what the computer has.

use crate::common::{catalogs_fetched, games_needing, offering_emulator};
use nexus_core::{Area, Command, Core, Gpu, Specs};
use nexus_testkit::{FakeHost, StubPlugin, TestCore};
use nexus_ui_api::VerdictVm;

/// An emulator whose games need `requires` (TOML for its `[requires]` section).
fn emulator(id: &str, requires: &str) -> StubPlugin {
    StubPlugin::from_toml(&format!(
        "id = \"{id}\"\nkind = \"emulator\"\nname = \"{id}\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\n\
         capabilities = [\"launcher\"]\n[requires]\n{requires}"
    ))
}

const PS5: &str = "vulkan = \"1.3\"\n[requires.recommended]\ncpu_threads = 12\nmemory_gb = 14.5\ngpu_memory_gb = 7.5\ndiscrete_gpu = true";

fn pc(threads: u32, memory_gb: f32, gpu: Option<Gpu>) -> Specs {
    Specs {
        cpu: "Test CPU".into(),
        cores: threads / 2,
        threads,
        memory_gb,
        avx2: Some(true),
        gpus: gpu.map(|g| vec![g]),
    }
}

fn card(vulkan: Option<(u32, u32)>, discrete: bool, memory_gb: Option<f32>) -> Gpu {
    Gpu { name: "Test GPU".into(), memory_gb, discrete, unified: false, vulkan }
}

async fn verdict(specs: Specs, requires: &str) -> VerdictVm {
    let t = FakeHost::new().hardware(specs).plugin(emulator("emu-a", requires)).boot_core().await;
    wait_for_check(&t.core).await;
    t.core.view().settings.hardware.verdicts.into_iter().next().expect("one verdict")
}

async fn wait_for_check(core: &Core) {
    nexus_testkit::until(core, Area::Hardware, |core| core.view().settings.hardware.known).await;
}

#[tokio::test]
async fn a_strong_pc_runs_well_and_lists_its_parts() {
    let gpu = card(Some((1, 3)), true, Some(24.0));
    let t = FakeHost::new().hardware(pc(32, 64.0, Some(gpu))).plugin(emulator("emu-a", PS5)).boot_core().await;
    wait_for_check(&t.core).await;
    let hw = t.core.view().settings.hardware;
    assert_eq!(hw.cpu, "Test CPU · 16 cores, 32 threads");
    assert_eq!(hw.memory, "64 GB");
    assert_eq!(hw.graphics, ["Test GPU · 24 GB · Vulkan 1.3"]);
    assert_eq!((hw.verdicts[0].headline.as_str(), hw.verdicts[0].why.as_str()), ("Should run well", ""));
}

#[tokio::test]
async fn an_old_driver_won_t_start_and_a_weak_part_runs_slowly() {
    let old = verdict(pc(32, 64.0, Some(card(Some((1, 2)), true, Some(24.0)))), PS5).await;
    assert_eq!((old.level.as_str(), old.why.as_str()), ("no", "No Vulkan 1.3"));

    let integrated = verdict(pc(32, 64.0, Some(card(Some((1, 3)), false, None))), PS5).await;
    assert_eq!((integrated.level.as_str(), integrated.why.as_str()), ("slow", "Integrated GPU"));

    let little_ram = verdict(pc(32, 8.0, Some(card(Some((1, 3)), true, Some(24.0)))), PS5).await;
    assert_eq!((little_ram.headline.as_str(), little_ram.why.as_str()), ("May run slowly", "8 GB RAM"));
}

#[tokio::test]
async fn what_the_system_doesn_t_say_is_unknown_not_a_failure() {
    let no_vulkan_info = verdict(pc(32, 64.0, Some(card(None, true, Some(24.0)))), PS5).await;
    assert_eq!(no_vulkan_info.level, "unknown");

    let no_graphics_check = verdict(pc(32, 64.0, None), PS5).await;
    assert_eq!(no_graphics_check.level, "unknown");

    // AVX2 is only checked where the system says.
    let mut specs = pc(16, 32.0, Some(card(Some((1, 3)), true, Some(8.0))));
    specs.avx2 = None;
    assert_eq!(verdict(specs.clone(), "avx2 = true").await.level, "good");
    specs.avx2 = Some(false);
    let no_avx2 = verdict(specs, "avx2 = true").await;
    assert_eq!((no_avx2.level.as_str(), no_avx2.why.as_str()), ("no", "No AVX2 in the processor"));
}

#[tokio::test]
async fn a_unified_gpu_is_judged_by_the_system_s_memory_not_by_being_built_in() {
    // An Apple Silicon Mac: the GPU is built in and shares 36 GB.
    let apple =
        Gpu { name: "Apple M4 Max".into(), memory_gb: None, discrete: false, unified: true, vulkan: Some((1, 3)) };
    let strong = verdict(pc(14, 36.0, Some(apple.clone())), PS5).await;
    assert_eq!((strong.level.as_str(), strong.why.as_str()), ("good", ""));

    // 8 GB of shared memory leaves the GPU about 6 GB: under the 7.5 GB a PS5 game wants.
    let small = verdict(pc(14, 8.0, Some(apple)), PS5).await;
    assert_eq!((small.level.as_str(), small.why.as_str()), ("slow", "6 GB GPU"));
}

const HASH: &str = "c12fe1c06bba254a9dc9f519b335aa7c1367a88a";

/// A strong PC: a PS5 game runs well on it.
fn strong() -> Specs {
    pc(32, 64.0, Some(card(Some((1, 3)), true, Some(24.0))))
}

/// A PC whose driver is too old for a PS5 game.
fn old_driver() -> Specs {
    pc(32, 64.0, Some(card(Some((1, 2)), true, Some(24.0))))
}

/// KytyPS5's catalog offers one 50 GB release of Astro, open on its Game Hub, on a host that
/// `build` changes first.
async fn astro_offer(build: impl FnOnce(FakeHost) -> FakeHost) -> (TestCore, tempfile::TempDir) {
    let home = tempfile::tempdir().unwrap();
    let t = build(FakeHost::new())
        .home(home.path())
        .plugin(offering_emulator(PS5))
        .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
        .boot_core()
        .await;
    let catalog = serde_json::json!({ "complete": true, "translation": { "language": "en" }, "topic_count": 1,
        "topics": [{ "id": "1", "title": "[PS5] Astro [EUR / v01.05]", "size": "50 GB",
            "magnet": format!("magnet:?xt=urn:btih:{HASH}"),
            "game_info": { "name": "Astro", "title_id": "PPSA01234", "region": "EUR", "version": "v01.05" } }] });
    t.http.serve("https://xyz.dev/c/ps5.json", catalog.to_string());
    wait_for_check(&t.core).await;
    t.core
        .dispatch(Command::SetConfig {
            plugin: nexus_api::PluginId::new("release-catalog").unwrap(),
            scope: nexus_api::ConfigScope::Emulator,
            value: serde_json::json!({ "catalog_url": "https://xyz.dev/c" }),
        })
        .await
        .unwrap();
    catalogs_fetched(&t.core).await;
    t.core.dispatch(Command::OpenGame("release-catalog:emu-kyty/PPSA01234".parse().unwrap())).await.unwrap();
    (t, home)
}

fn on_this_pc(t: &TestCore) -> Option<(String, String, String)> {
    let pc = t.core.view().detail.expect("the Game Hub").this_pc?;
    Some((pc.level, pc.headline, pc.why))
}

#[tokio::test]
async fn an_installed_game_says_how_it_runs_on_this_pc() {
    let t = FakeHost::new().hardware(old_driver()).plugin(games_needing("emu-a", PS5, vec!["Astro"])).boot_core().await;
    wait_for_check(&t.core).await;
    t.core.dispatch(Command::OpenGame("emu-a:G0".parse().unwrap())).await.unwrap();
    assert_eq!(on_this_pc(&t), Some(("no".into(), "Won't start".into(), "No Vulkan 1.3".into())));
}

#[tokio::test]
async fn an_offer_says_how_its_emulator_runs_on_this_pc() {
    let (t, _home) = astro_offer(|h| h.hardware(strong())).await;
    assert_eq!(on_this_pc(&t), Some(("good".into(), "Should run well".into(), String::new())));
    let detail = t.core.view().detail.unwrap();
    assert_eq!(detail.pc_warning, "", "nothing to warn of");
    assert_eq!(detail.releases[0].no_room, "");
}

#[tokio::test]
async fn an_offer_bigger_than_the_free_space_says_so_on_its_card_and_its_release() {
    let (t, _home) = astro_offer(|h| h.hardware(strong()).free_space(10_000_000_000)).await;
    assert_eq!(on_this_pc(&t), Some(("no".into(), "Not enough space".into(), "10.0 GB free".into())));
    let detail = t.core.view().detail.unwrap();
    // The catalog's 50 GB are binary: 53.7 billion bytes.
    assert_eq!(
        detail.releases[0].no_room, "Not enough free space in the download folder: 53.7 GB needed, 10.0 GB free",
        "the same words as the download's refusal"
    );
}

#[tokio::test]
async fn an_offer_that_won_t_start_here_says_why_before_the_space() {
    let (t, _home) = astro_offer(|h| h.hardware(old_driver()).free_space(10_000_000_000)).await;
    assert_eq!(on_this_pc(&t), Some(("no".into(), "Won't start".into(), "No Vulkan 1.3".into())));
    assert_eq!(t.core.view().detail.unwrap().pc_warning, "This PC can't run PS5 games: No Vulkan 1.3");
}

#[tokio::test]
async fn before_the_check_and_without_a_check_there_is_no_card() {
    let (t, _home) = astro_offer(|h| h).await;
    assert_eq!(on_this_pc(&t), None);
}

/// The PC catalog offers one game with `topics`, open on its Game Hub, on a strong Mac.
async fn pc_offer_on_a_mac(topics: serde_json::Value) -> (TestCore, tempfile::TempDir) {
    let home = tempfile::tempdir().unwrap();
    let t = FakeHost::new()
        .home(home.path())
        .platform(Some(nexus_api::Platform::Macos))
        .hardware(strong())
        .built_in(nexus_api::PluginBox::code(emu_pc::PcPlugin))
        .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
        .boot_core()
        .await;
    let count = topics.as_array().unwrap().len();
    let catalog = serde_json::json!({ "complete": true, "translation": { "language": "en" }, "topic_count": count,
        "topics": topics });
    t.http.serve("https://xyz.dev/c/pc.json", catalog.to_string());
    wait_for_check(&t.core).await;
    t.core
        .dispatch(Command::SetConfig {
            plugin: nexus_api::PluginId::new("release-catalog").unwrap(),
            scope: nexus_api::ConfigScope::Emulator,
            value: serde_json::json!({ "catalog_url": "https://xyz.dev/c" }),
        })
        .await
        .unwrap();
    catalogs_fetched(&t.core).await;
    t.core.dispatch(Command::Navigate(nexus_core::Route::Catalog)).await.unwrap();
    let offers = t.core.view().catalog.games.games;
    assert_eq!(offers.len(), 1, "one game: {offers:?}");
    t.core.dispatch(Command::OpenGame(offers[0].key.parse().unwrap())).await.unwrap();
    (t, home)
}

fn pc_topic(id: &str, title: &str, hash: char) -> serde_json::Value {
    let magnet = format!("magnet:?xt=urn:btih:{}", hash.to_string().repeat(40));
    serde_json::json!({ "id": id, "title": title, "size": "1 GB", "magnet": magnet, "game_info": {} })
}

#[tokio::test]
async fn a_pc_offer_with_no_release_for_this_system_won_t_start_though_its_emulator_needs_nothing() {
    let (t, _home) =
        pc_offer_on_a_mac(serde_json::json!([pc_topic("1", "Penguins (v1.2) [amd64] [Native]", 'a')])).await;
    let detail = t.core.view().detail.unwrap();
    assert_eq!(detail.releases[0].not_here, "This is a Linux release: it can't run on a Mac");
    assert_eq!(on_this_pc(&t), Some(("no".into(), "Won't start".into(), "Linux release".into())));
    assert_eq!(detail.pc_warning, "", "the release row says why");
}

#[tokio::test]
async fn a_pc_offer_with_a_release_for_this_system_should_run_well() {
    let (t, _home) = pc_offer_on_a_mac(serde_json::json!([
        pc_topic("1", "Penguins (v1.2) [amd64] [Native]", 'a'),
        pc_topic("2", "Penguins (v1.2) [Windows]", 'b'),
    ]))
    .await;
    assert_eq!(t.core.view().detail.unwrap().releases.len(), 2);
    assert_eq!(on_this_pc(&t), Some(("good".into(), "Should run well".into(), String::new())));
}

/// The notices of a run on `specs` whose config already holds `noticed` for `app.pc_noticed`.
async fn notices_on(specs: Specs, noticed: serde_json::Value) -> (Vec<String>, String, serde_json::Value) {
    let host = FakeHost::new().hardware(specs).plugin(emulator("emu-a", PS5)).boot().await;
    host.kernel.config().set(&["app", "welcomed"], serde_json::Value::Bool(true)).unwrap();
    host.kernel.config().set(&["app", "pc_noticed"], noticed).unwrap();
    let core = Core::start(host.kernel.clone()).await;
    wait_for_check(&core).await;
    let view = core.view();
    (view.notices, view.notice_opens, host.kernel.config().get(&["app", "pc_noticed"]).clone())
}

#[tokio::test]
async fn a_pc_that_can_t_start_an_emulator_s_games_says_so_once() {
    let (notices, opens, noticed) = notices_on(old_driver(), serde_json::Value::Null).await;
    assert_eq!(notices, ["This PC can't run emu-a games: No Vulkan 1.3"]);
    assert_eq!(opens, "Open Settings");

    let (again, _, _) = notices_on(old_driver(), noticed.clone()).await;
    assert!(again.is_empty(), "not again on the same PC: {again:?}");

    let mut other = old_driver();
    other.gpus.as_mut().unwrap()[0].name = "Other GPU".into();
    let (changed, _, _) = notices_on(other, noticed).await;
    assert_eq!(changed.len(), 1, "again when the PC changes");

    let (strong, _, _) = notices_on(strong(), serde_json::Value::Null).await;
    assert!(strong.is_empty(), "nothing to say on a PC that runs them");
}

#[tokio::test]
async fn an_offer_says_not_enough_space_from_its_best_release_not_its_newest_upload() {
    let home = tempfile::tempdir().unwrap();
    let t = FakeHost::new()
        .hardware(strong())
        .free_space(10_000_000_000)
        .home(home.path())
        .plugin(offering_emulator(PS5))
        .built_in(nexus_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
        .boot_core()
        .await;
    // The newest upload fits; the best release, the highest version, doesn't.
    let catalog = serde_json::json!({ "complete": true, "translation": { "language": "en" }, "topic_count": 2,
        "topics": [
            { "id": "2", "title": "[PS5] Astro [EUR / v01.00]", "size": "5 GB", "published_at": "2026-10-10",
              "magnet": format!("magnet:?xt=urn:btih:{}", "b".repeat(40)),
              "game_info": { "name": "Astro", "title_id": "PPSA01234", "region": "EUR", "version": "v01.00" } },
            { "id": "1", "title": "[PS5] Astro [EUR / v01.05]", "size": "50 GB", "published_at": "2026-09-01",
              "magnet": format!("magnet:?xt=urn:btih:{HASH}"),
              "game_info": { "name": "Astro", "title_id": "PPSA01234", "region": "EUR", "version": "v01.05" } }] });
    t.http.serve("https://xyz.dev/c/ps5.json", catalog.to_string());
    wait_for_check(&t.core).await;
    t.core
        .dispatch(Command::SetConfig {
            plugin: nexus_api::PluginId::new("release-catalog").unwrap(),
            scope: nexus_api::ConfigScope::Emulator,
            value: serde_json::json!({ "catalog_url": "https://xyz.dev/c" }),
        })
        .await
        .unwrap();
    catalogs_fetched(&t.core).await;
    t.core.dispatch(Command::OpenGame("release-catalog:emu-kyty/PPSA01234".parse().unwrap())).await.unwrap();
    assert_eq!(on_this_pc(&t), Some(("no".into(), "Not enough space".into(), "10.0 GB free".into())));
}
