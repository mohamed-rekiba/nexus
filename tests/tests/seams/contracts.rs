//! Seam 2: the capability contracts catch a plugin that breaks a rule. The KytyPS5 and
//! shadPS4 plugins run the full suites in `kyty.rs` and `shadps4.rs`.

use nexus_api::PluginId;
use nexus_testkit::FakeHost;
use nexus_testkit::contract;

/// A catalog that lists one game twice.
struct Twice;

#[nexus_api::async_trait]
impl nexus_api::Catalog for Twice {
    async fn list(
        &self,
        _: nexus_api::PageRequest,
    ) -> nexus_api::PluginResult<nexus_api::Page<nexus_api::GameSummary>> {
        let game = nexus_api::GameSummary {
            id: "same".into(),
            title: "Same".into(),
            platform: "PS4".into(),
            cover: None,
            tags: vec![],
            release_year: None,
            title_id: None,
            content_id: None,
        };
        Ok(nexus_api::Page { items: vec![game.clone(), game], total: None })
    }
    async fn search(&self, _: &nexus_api::SearchQuery) -> nexus_api::PluginResult<Vec<nexus_api::GameSummary>> {
        Ok(vec![])
    }
    async fn details(&self, game: &str) -> nexus_api::PluginResult<nexus_api::GameDetails> {
        Err(nexus_api::PluginError::NotFound(game.into()))
    }
    async fn artwork(
        &self,
        _: &str,
        _: nexus_api::ArtworkKind,
    ) -> nexus_api::PluginResult<Option<nexus_api::ImageSource>> {
        Ok(None)
    }
}

#[tokio::test]
#[should_panic(expected = "contract catalog: game ids must be unique")]
async fn the_contract_catches_a_plugin_that_breaks_a_rule() {
    let plugin = nexus_testkit::StubPlugin::from_toml(
        "id = \"twice\"\nkind = \"emulator\"\nname = \"Twice\"\nversion = \"1.0.0\"\nhost_api = \"^0.1\"\ncapabilities = [\"catalog\"]",
    )
    .provides(|_| Ok(nexus_api::Provided { catalog: Some(std::sync::Arc::new(Twice)), ..Default::default() }));
    let test = FakeHost::new().plugin(plugin).boot_core().await;
    contract::emulator(&test, &PluginId::new("twice").unwrap()).await;
}
