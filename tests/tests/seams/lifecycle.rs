//! Seam 1, core app API: plugin lifecycle, isolation and the plugin context.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use nexus_api::{
    Catalog, Event, GameDetails, GameSummary, HttpRequest, Page, PageRequest, PluginError, PluginId, PluginResult,
    Provided, RelPath, SearchQuery,
};
use nexus_core::{Clock, CoreError, LoadError, PluginStatus};
use nexus_testkit::{FakeHost, StubPlugin};

fn id(s: &str) -> PluginId {
    PluginId::new(s).unwrap()
}

fn manifest(id: &str, extra: &str) -> String {
    format!("id = \"{id}\"\nkind = \"emulator\"\nname = \"{id}\"\nversion = \"1.0.0\"\nhost_api = \"^0.2\"\n{extra}")
}

struct EmptyCatalog;

#[nexus_api::async_trait]
impl Catalog for EmptyCatalog {
    async fn list(&self, _: PageRequest) -> PluginResult<Page<GameSummary>> {
        Ok(Page { items: vec![], total: Some(0) })
    }
    async fn search(&self, _: &SearchQuery) -> PluginResult<Vec<GameSummary>> {
        Ok(vec![])
    }
    async fn details(&self, game: &str) -> PluginResult<GameDetails> {
        Err(PluginError::NotFound(game.into()))
    }
    async fn artwork(&self, _: &str, _: nexus_api::ArtworkKind) -> PluginResult<Option<nexus_api::ImageSource>> {
        Ok(None)
    }
}

fn with_catalog(_: nexus_api::PluginContext) -> PluginResult<Provided> {
    Ok(Provided { catalog: Some(Arc::new(EmptyCatalog)), ..Provided::default() })
}

#[tokio::test]
async fn a_lazy_plugin_inits_once_on_first_use() {
    let stub = StubPlugin::from_toml(&manifest("lazy", "capabilities = [\"catalog\"]")).provides(with_catalog);
    let host = FakeHost::new().plugin(stub.clone()).boot().await;
    assert_eq!(stub.init_calls(), 0, "nothing runs at boot");

    let provided = host.kernel.provided(&id("lazy")).await.unwrap();
    assert!(provided.catalog.is_some());
    host.kernel.provided(&id("lazy")).await.unwrap();
    assert_eq!(stub.init_calls(), 1);
}

#[tokio::test]
async fn an_eager_plugin_inits_at_boot() {
    let stub = StubPlugin::from_toml(&manifest("eager", "activation = \"eager\""));
    let host = FakeHost::new().plugin(stub.clone()).boot().await;

    assert_eq!(stub.init_calls(), 1);
    assert_eq!(host.kernel.plugins().await[0].status, PluginStatus::Active);
}

#[tokio::test]
async fn a_plugin_that_panics_is_marked_failed_and_the_others_keep_working() {
    let bad = StubPlugin::from_toml(&manifest("bad", "")).panics_on_init();
    let good = StubPlugin::from_toml(&manifest("good", ""));
    let host = FakeHost::new().plugin(bad.clone()).plugin(good).boot().await;
    let mut events = host.kernel.events().stream();

    let err = host.kernel.provided(&id("bad")).await.unwrap_err();
    assert!(
        matches!(&err, CoreError::PluginUnavailable { reason, .. } if reason.contains("panicked on purpose")),
        "{err}"
    );
    assert!(host.kernel.provided(&id("good")).await.is_ok());
    assert!(matches!(events.next().await, Some(Event::PluginFailed { plugin, .. }) if plugin == id("bad")));

    // A failed plugin is not retried.
    assert!(host.kernel.provided(&id("bad")).await.is_err());
    assert_eq!(bad.init_calls(), 1);
}

#[tokio::test]
async fn a_plugin_that_hangs_times_out() {
    let stub = StubPlugin::from_toml(&manifest("slow", "")).hangs_on_init();
    let host = FakeHost::new().call_timeout(Duration::from_millis(50)).plugin(stub).boot().await;

    let err = host.kernel.provided(&id("slow")).await.unwrap_err();
    assert!(
        matches!(&err, CoreError::PluginUnavailable { reason, .. } if reason.contains("timed out after 50 ms")),
        "{err}"
    );
}

#[tokio::test]
async fn an_eager_plugin_that_fails_appears_in_the_boot_report() {
    let stub = StubPlugin::from_toml(&manifest("broken", "activation = \"eager\""))
        .fails_on_init(PluginError::Precondition("firmware missing".into()));
    let host = FakeHost::new().plugin(stub).boot().await;

    assert_eq!(
        host.report().rejection(&id("broken")),
        Some(&LoadError::Init(PluginError::Precondition("firmware missing".into())))
    );
}

#[tokio::test]
async fn a_plugin_must_provide_exactly_what_its_manifest_declares() {
    let stub =
        StubPlugin::from_toml(&manifest("liar", "capabilities = [\"catalog\", \"launcher\"]")).provides(with_catalog);
    let host = FakeHost::new().plugin(stub.clone()).boot().await;

    let err = host.kernel.provided(&id("liar")).await.unwrap_err();
    assert!(matches!(&err, CoreError::PluginUnavailable { reason, .. } if reason.contains("Launcher")), "{err}");
    assert_eq!(stub.shutdown_calls(), 1, "the host shuts down what it can't use");
}

#[tokio::test]
async fn deactivate_is_idempotent_and_shutdown_runs_once() {
    let stub = StubPlugin::from_toml(&manifest("once", ""));
    let host = FakeHost::new().plugin(stub.clone()).boot().await;
    host.kernel.activate(&id("once")).await.unwrap();

    host.kernel.deactivate(&id("once")).await.unwrap();
    host.kernel.deactivate(&id("once")).await.unwrap();
    assert_eq!(stub.shutdown_calls(), 1);

    // Deactivating a plugin that never ran does nothing.
    let idle = StubPlugin::from_toml(&manifest("idle", ""));
    let host = FakeHost::new().plugin(idle.clone()).boot().await;
    host.kernel.deactivate(&id("idle")).await.unwrap();
    assert_eq!(idle.shutdown_calls(), 0);
}

#[tokio::test]
async fn a_shutdown_that_hangs_is_cut_off_by_the_timeout() {
    let stub = StubPlugin::from_toml(&manifest("stuck", "")).hangs_on_shutdown();
    let host = FakeHost::new().plugin(stub).boot().await;
    host.kernel.activate(&id("stuck")).await.unwrap();

    let started = host.clock.now();
    host.kernel.deactivate(&id("stuck")).await.unwrap();
    let took = host.clock.now() - started;
    assert!(took >= Duration::from_millis(200) && took < Duration::from_secs(2), "the 200 ms limit: {took:?}");
    assert_eq!(host.kernel.plugins().await[0].status, PluginStatus::Loaded);
}

#[tokio::test]
async fn deactivating_a_dependency_deactivates_its_dependents_first() {
    let lib = StubPlugin::from_toml(&manifest("lib", ""));
    let app = StubPlugin::from_toml(&manifest("app", "[[dependencies]]\nid = \"lib\""));
    let host = FakeHost::new().plugin(lib.clone()).plugin(app.clone()).boot().await;
    host.kernel.activate(&id("app")).await.unwrap();
    assert_eq!(lib.init_calls(), 1, "activating app activates lib first");

    let mut events = host.kernel.events().stream();
    host.kernel.deactivate(&id("lib")).await.unwrap();
    assert_eq!(events.next().await, Some(Event::PluginDeactivated { plugin: id("app") }));
    assert_eq!(events.next().await, Some(Event::PluginDeactivated { plugin: id("lib") }));
}

#[tokio::test]
async fn safe_mode_loads_built_in_plugins_only() {
    let built_in = StubPlugin::from_toml(&manifest("built-in", ""));
    // Third-party plugins are data-only themes.
    let outsider = StubPlugin::from_toml(
        "id = \"outsider\"\nkind = \"theme\"\nname = \"outsider\"\nversion = \"1.0.0\"\nhost_api = \"^0.2\"\n\
         capabilities = [\"theme\"]\n[runtime]\ntype = \"data\"",
    );
    let host = FakeHost::new().safe_mode().plugin(built_in).third_party(outsider.boxed()).boot().await;

    assert_eq!(host.report().order, vec![id("built-in")]);
    assert_eq!(host.report().rejection(&id("outsider")), Some(&LoadError::SafeMode));
}

#[tokio::test]
async fn the_context_scopes_storage_network_and_events_to_the_plugin() {
    let seen = Arc::new(std::sync::Mutex::new(None));
    let seen_in_init = seen.clone();
    let stub = StubPlugin::from_toml(&manifest("scoped", "[permissions]\nnetwork = [\"allowed.example\"]")).provides(
        move |ctx| {
            *seen_in_init.lock().unwrap() = Some(ctx);
            Ok(Provided::default())
        },
    );
    let host = FakeHost::new().plugin(stub).boot().await;
    host.http.serve("https://allowed.example/list.json", "[]");
    host.kernel.activate(&id("scoped")).await.unwrap();
    let ctx = seen.lock().unwrap().clone().unwrap();

    // Storage lands in the plugin's own folder.
    ctx.storage().write(&RelPath::new("notes/a.txt").unwrap(), b"hi").await.unwrap();
    assert!(host.data_dir().join("plugins/scoped/notes/a.txt").is_file());

    // HTTP reaches allowed hosts only.
    let ok = ctx.http().send(HttpRequest::get("https://allowed.example/list.json")).await.unwrap();
    assert_eq!(ok.status, 200);
    let denied = ctx.http().send(HttpRequest::get("https://other.example/")).await.unwrap_err();
    assert!(matches!(denied, PluginError::PermissionDenied(_)), "{denied:?}");

    // Published events carry the plugin's id.
    let mut events = host.kernel.events().stream();
    ctx.events().publish("hello", serde_json::json!({"n": 1})).unwrap();
    assert_eq!(
        events.next().await,
        Some(Event::Custom { plugin: id("scoped"), topic: "hello".into(), payload: serde_json::json!({"n": 1}) })
    );
}

#[tokio::test]
async fn a_plugin_call_s_limit_runs_on_the_clock_and_not_before() {
    let stub = StubPlugin::from_toml(&manifest("stuck", "")).hangs_on_shutdown();
    let host = FakeHost::new().plugin(stub).boot().await;
    host.kernel.activate(&id("stuck")).await.unwrap();

    let kernel = host.kernel.clone();
    let off = tokio::spawn(async move { kernel.deactivate(&id("stuck")).await });
    // The fake host gives a shutdown 200 ms.
    host.clock.advance(Duration::from_millis(150)).await;
    assert!(!off.is_finished(), "the shutdown still runs");
    host.clock.advance(Duration::from_millis(60)).await;
    assert!(off.is_finished(), "the limit cut it off");
    off.await.unwrap().unwrap();
}
