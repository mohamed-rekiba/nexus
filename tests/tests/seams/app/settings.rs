//! Settings the core keeps and checks: the plugin's settings and the preferences.

use super::*;

#[tokio::test]
async fn settings_are_checked_by_the_plugin_before_they_are_stored() {
    struct Strict;
    #[async_trait]
    impl Configurator for Strict {
        async fn schema(&self, _: &ConfigScope) -> PluginResult<serde_json::Value> {
            Ok(serde_json::json!({"type": "object", "properties": {"scale": {"type": "integer", "title": "Scale"}}}))
        }
        async fn defaults(&self, _: &ConfigScope) -> PluginResult<serde_json::Value> {
            Ok(serde_json::json!({"scale": 1}))
        }
        async fn validate(&self, _: &ConfigScope, value: &serde_json::Value) -> PluginResult<()> {
            match value.get("scale").and_then(|s| s.as_i64()) {
                Some(1..=4) => Ok(()),
                _ => Err(PluginError::InvalidInput("scale must be 1 to 4".into())),
            }
        }
    }
    let plugin = StubPlugin::from_toml(
        "id = \"cfg\"\nkind = \"emulator\"\nname = \"Cfg\"\nversion = \"1.0.0\"\nhost_api = \"^0.2\"\ncapabilities = [\"configurator\"]",
    )
    .provides(|_| Ok(Provided { configurator: Some(Arc::new(Strict)), ..Provided::default() }));
    let t = FakeHost::new().plugin(plugin).boot_core().await;
    let set = |v| Command::SetConfig { plugin: id("cfg"), scope: ConfigScope::Emulator, value: v };

    let err = t.core.dispatch(set(serde_json::json!({"scale": 9}))).await.unwrap_err();
    assert!(err.to_string().contains("scale must be 1 to 4"), "{err}");
    t.core.dispatch(set(serde_json::json!({"scale": 3}))).await.unwrap();
    assert_eq!(t.core.config_value(&id("cfg"), &ConfigScope::Emulator).await.unwrap(), serde_json::json!({"scale": 3}));
}

#[tokio::test]
async fn the_size_of_everything_is_saved_and_stays_between_0_8_and_1_4() {
    let t = FakeHost::new().boot_core().await;
    assert_eq!(t.core.view().settings.scale, 1.0, "the theme's own size by default");
    t.core.dispatch(Command::Pref(Pref::Scale, 1.2.into())).await.unwrap();
    assert_eq!(t.core.view().settings.scale, 1.2);
    t.core.dispatch(Command::Pref(Pref::Scale, 3.0.into())).await.unwrap();
    assert_eq!(t.core.view().settings.scale, 1.4);
    t.core.dispatch(Command::Pref(Pref::Scale, 0.1.into())).await.unwrap();
    assert_eq!(t.core.view().settings.scale, 0.8);
    assert!(t.core.dispatch(Command::Pref(Pref::Scale, f32::NAN.into())).await.is_err());
    assert_eq!(t.core.view().settings.scale, 0.8);
}

#[tokio::test]
async fn start_at_login_goes_through_the_desktop_and_stays_off_when_the_system_refuses() {
    let t = FakeHost::new().boot_core().await;
    let settings = t.core.view().settings;
    assert!(settings.can_start_at_login);
    assert!(!settings.start_at_login, "off by default");
    t.core.dispatch(Command::Pref(Pref::StartAtLogin, true.into())).await.unwrap();
    assert!(t.core.view().settings.start_at_login);
    t.core.dispatch(Command::Pref(Pref::StartAtLogin, false.into())).await.unwrap();
    assert!(!t.core.view().settings.start_at_login);
    assert_eq!(t.login_session.logins(), [true, false]);

    t.login_session.refuse_login("reg.exe: access is denied");
    let refused = t.core.dispatch(Command::Pref(Pref::StartAtLogin, true.into())).await.unwrap_err();
    assert!(refused.to_string().contains("access is denied"), "{refused}");
    let settings = t.core.view().settings;
    assert!(!settings.start_at_login, "it stays off");
    assert_eq!(settings.start_at_login_error, "reg.exe: access is denied");
}

#[tokio::test]
async fn a_command_doesn_t_set_the_preferences_the_core_keeps() {
    let t = FakeHost::new().boot_core().await;
    let refused = t.core.dispatch(Command::Pref(Pref::Theme, "fixture-grid".into())).await.unwrap_err();
    assert!(matches!(refused, CoreError::Invalid(_)), "{refused}");
    assert_eq!(t.core.prefs().theme, None, "the UI host switches themes, through ThemeSwitched");
    assert!(t.core.dispatch(Command::Pref(Pref::Sounds, "off".into())).await.is_err(), "a flag takes on or off");
    assert!(t.core.prefs().sounds);
}

#[tokio::test]
async fn start_at_login_is_hidden_when_this_build_isn_t_installed() {
    let host = FakeHost::new();
    host.login_session().no_login();
    let t = host.boot_core().await;
    assert!(!t.core.view().settings.can_start_at_login);
    assert!(t.core.dispatch(Command::Pref(Pref::StartAtLogin, true.into())).await.is_err());
    assert!(t.login_session.logins().is_empty(), "the desktop is not asked");
}
