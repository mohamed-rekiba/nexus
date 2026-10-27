//! The preferences subsystem: the app's own one-value settings, typed.
//!
//! Each [`Pref`] is one `app.<key>` in the config, with the same key and value as before:
//! `app.sounds`, `app.scale` and so on. Its spec says what values it takes, what it is when
//! unset, and which area changes with it. A value the player sets is checked against that spec
//! (a size goes to the nearest end of its range), and a saved value that doesn't fit reads as
//! the default.
//!
//! The settings that belong to another area stay there: the torrents subsystem keeps
//! `app.seed_downloads`, and the lists and maps (`app.folders`, `app.playtime`, …) stay with
//! their areas.
//!
//! The subsystem follows the pattern in [`crate::torrents`]. The config is its store: it reads
//! through to the config, so it keeps no copy that could drift. Its only state of its own is
//! why the system refused "Start at login" last.

use std::sync::{Arc, Mutex, MutexGuard};

use nexus_api::PluginId;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::changes::{Area, Notifier};
use crate::config::ConfigService;
use crate::vpn::NetworkSource;
use crate::{CoreError, CoreResult, LoginSession};

/// The smallest and the largest size of everything ([`Pref::Scale`]).
pub const MIN_SCALE: f32 = 0.8;
pub const MAX_SCALE: f32 = 1.4;

/// One of the app's own settings. Its name in JSON is its config key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pref {
    /// Theme sounds on or off. On by default.
    Sounds,
    /// The launcher's window full screen, or a normal window. On by default, as in the old
    /// launcher.
    Fullscreen,
    /// Whether the launcher comes back to the front when a game closes. On by default, as in
    /// the old launcher.
    ReturnOnExit,
    /// The size of everything the theme draws, from [`MIN_SCALE`] to [`MAX_SCALE`] (1 is the
    /// theme's own size). A value outside that range goes to its nearest end.
    Scale,
    /// The launcher starts when the player logs in. Off by default. When the system refuses,
    /// the setting stays as it was and says why.
    StartAtLogin,
    /// Emulator updates install by themselves. On by default.
    AutoUpdate,
    /// The monitor the launcher goes on, by name; null for the one it is on.
    Monitor,
    /// The player pressed Start on the Welcome screen: it doesn't show again.
    Welcomed,
    /// "Download only through a VPN". Off by default. On is refused where the system can't
    /// bind downloads to a VPN, and says why.
    VpnOnly,
    /// The VPN downloads use, by its name. Empty: the first connected one, where the system
    /// finds VPNs.
    Vpn,
    /// The theme the player chose. The UI host sets it once the theme is on screen
    /// (`Command::ThemeSwitched`).
    Theme,
    /// The PC the "games won't start here" notice was last for. The core sets it.
    PcNoticed,
    /// Where the launcher's releases are, "owner/name". Unset: Nexus's own; empty: no checks.
    /// Set by hand in the config.
    UpdateRepo,
}

/// What values a [`Pref`] takes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PrefKind {
    /// On or off.
    Flag,
    /// A number from `min` to `max`, saved with two decimals.
    Size { min: f32, max: f32 },
    /// Any text.
    Text,
    /// A name, or null for none. Empty text is none.
    Name,
}

/// What a [`Pref`] is: its values, its value when unset, and whether the player sets it.
#[derive(Clone, Debug, PartialEq)]
pub struct PrefSpec {
    pub kind: PrefKind,
    pub default: Value,
    /// The player sets it with `Command::Pref`. The others are set by the core or by hand.
    pub by_player: bool,
}

impl Pref {
    /// Every preference.
    pub const ALL: [Pref; 13] = [
        Pref::Sounds,
        Pref::Fullscreen,
        Pref::ReturnOnExit,
        Pref::Scale,
        Pref::StartAtLogin,
        Pref::AutoUpdate,
        Pref::Monitor,
        Pref::Welcomed,
        Pref::VpnOnly,
        Pref::Vpn,
        Pref::Theme,
        Pref::PcNoticed,
        Pref::UpdateRepo,
    ];

    /// Its key under `app` in the config.
    pub fn key(self) -> &'static str {
        match self {
            Pref::Sounds => "sounds",
            Pref::Fullscreen => "fullscreen",
            Pref::ReturnOnExit => "return_on_exit",
            Pref::Scale => "scale",
            Pref::StartAtLogin => "start_at_login",
            Pref::AutoUpdate => "auto_update",
            Pref::Monitor => "monitor",
            Pref::Welcomed => "welcomed",
            Pref::VpnOnly => "vpn_only",
            Pref::Vpn => "vpn",
            Pref::Theme => "theme",
            Pref::PcNoticed => "pc_noticed",
            Pref::UpdateRepo => "update_repo",
        }
    }

    pub fn spec(self) -> PrefSpec {
        let (kind, default, by_player) = match self {
            Pref::Sounds | Pref::Fullscreen | Pref::ReturnOnExit | Pref::AutoUpdate => {
                (PrefKind::Flag, Value::Bool(true), true)
            }
            Pref::StartAtLogin | Pref::Welcomed | Pref::VpnOnly => (PrefKind::Flag, Value::Bool(false), true),
            Pref::Scale => (PrefKind::Size { min: MIN_SCALE, max: MAX_SCALE }, Value::from(1.0), true),
            Pref::Monitor => (PrefKind::Name, Value::Null, true),
            Pref::Vpn => (PrefKind::Text, Value::from(""), true),
            Pref::Theme | Pref::PcNoticed => (PrefKind::Name, Value::Null, false),
            Pref::UpdateRepo => (PrefKind::Text, Value::from(crate::launcher_update::DEFAULT_REPO), false),
        };
        PrefSpec { kind, default, by_player }
    }

    /// The area that changes with it.
    pub fn area(self) -> Area {
        match self {
            Pref::VpnOnly | Pref::Vpn => Area::Vpn,
            _ => Area::Settings,
        }
    }

    /// `value` as it is saved: a size goes to the nearest end of its range, with two
    /// decimals, and empty text is no name. A value of the wrong type is refused.
    pub fn check(self, value: Value) -> CoreResult<Value> {
        let refused = |value: &Value| CoreError::Invalid(format!("{value} isn't a value for {}", self.key()));
        match self.spec().kind {
            PrefKind::Size { .. } => {
                let size = self.fit(&value).ok_or_else(|| refused(&value))?;
                let size = size.as_f64().expect("a fitted size is a number");
                // Two decimals: 1.2, not the f32's 1.2000000476837158.
                Ok(Value::from((size * 100.0).round() / 100.0))
            }
            PrefKind::Name if value.as_str() == Some("") => Ok(Value::Null),
            _ => self.fit(&value).ok_or_else(|| refused(&value)),
        }
    }

    /// The value of a saved `saved`: the default when it is unset or of the wrong type.
    pub fn read(self, saved: &Value) -> Value {
        self.fit(saved).unwrap_or_else(|| self.spec().default)
    }

    /// Reads it from `config`.
    pub fn load(self, config: &ConfigService) -> Value {
        self.read(&config.get(&["app", self.key()]))
    }

    /// `value` when it has the right type, a size within its range.
    fn fit(self, value: &Value) -> Option<Value> {
        match (self.spec().kind, value) {
            (PrefKind::Flag, Value::Bool(_)) | (PrefKind::Text, Value::String(_)) => Some(value.clone()),
            (PrefKind::Name, Value::Null | Value::String(_)) => Some(value.clone()),
            (PrefKind::Size { min, max }, Value::Number(n)) => {
                let size = n.as_f64()? as f32;
                size.is_finite().then(|| Value::from(f64::from(size.clamp(min, max))))
            }
            _ => None,
        }
    }
}

/// The preferences, read whole and typed.
#[derive(Clone, Debug, PartialEq)]
pub struct PreferencesSnapshot {
    pub sounds: bool,
    pub fullscreen: bool,
    pub return_on_exit: bool,
    pub scale: f32,
    pub start_at_login: bool,
    /// Why the system refused "Start at login" last; `None` once it agreed.
    pub start_at_login_error: Option<String>,
    pub auto_update: bool,
    pub monitor: Option<String>,
    pub welcomed: bool,
    /// The player's setting. Downloads go only through the VPN when it is on and the system
    /// can bind them to one: see [`crate::vpn::vpn_only`].
    pub vpn_only: bool,
    pub vpn: String,
    /// The theme the player chose last, when its id is valid.
    pub theme: Option<PluginId>,
}

/// The ports the subsystem uses, and nothing else.
pub(crate) struct PrefPorts {
    /// For "Start at login".
    pub login_session: Arc<dyn LoginSession>,
    /// For "Download only through a VPN": whether the system can bind downloads to a VPN.
    pub network: NetworkSource,
}

/// The app's own settings. The config holds them; this checks what goes in and types what
/// comes out.
pub(crate) struct Preferences {
    config: Arc<ConfigService>,
    ports: PrefPorts,
    notifier: Notifier,
    login_error: Mutex<Option<String>>,
}

impl Preferences {
    pub fn new(config: Arc<ConfigService>, ports: PrefPorts, notifier: Notifier) -> Self {
        Self { config, ports, notifier, login_error: Mutex::new(None) }
    }

    fn login_error(&self) -> MutexGuard<'_, Option<String>> {
        self.login_error.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn get(&self, pref: Pref) -> Value {
        pref.load(&self.config)
    }

    pub fn snapshot(&self) -> PreferencesSnapshot {
        let flag = |pref: Pref| self.get(pref).as_bool().expect("a flag reads as a bool");
        let name = |pref: Pref| self.get(pref).as_str().map(str::to_owned);
        PreferencesSnapshot {
            sounds: flag(Pref::Sounds),
            fullscreen: flag(Pref::Fullscreen),
            return_on_exit: flag(Pref::ReturnOnExit),
            scale: self.get(Pref::Scale).as_f64().expect("a size reads as a number") as f32,
            start_at_login: flag(Pref::StartAtLogin),
            start_at_login_error: self.login_error().clone(),
            auto_update: flag(Pref::AutoUpdate),
            monitor: name(Pref::Monitor),
            welcomed: flag(Pref::Welcomed),
            vpn_only: flag(Pref::VpnOnly),
            vpn: name(Pref::Vpn).unwrap_or_default(),
            theme: name(Pref::Theme).and_then(|id| PluginId::new(id).ok()),
        }
    }

    /// Sets a preference the player sets. "Start at login" asks the system first, and "Download
    /// only through a VPN" is refused where the system can't bind downloads to a VPN.
    pub async fn set(&self, pref: Pref, value: Value) -> CoreResult<()> {
        if !pref.spec().by_player {
            return Err(CoreError::Invalid(format!("the player doesn't set {}", pref.key())));
        }
        let value = pref.check(value)?;
        let on = value == Value::Bool(true);
        match pref {
            Pref::StartAtLogin => self.start_at_login(on).await?,
            Pref::VpnOnly if on => {
                if let Some(why) = self.ports.network.get().problem() {
                    return Err(CoreError::Invalid(why));
                }
            }
            _ => {}
        }
        self.write(pref, value)
    }

    /// Saves a preference the core keeps, such as the theme the player chose.
    pub fn save(&self, pref: Pref, value: Value) -> CoreResult<()> {
        let value = pref.check(value)?;
        self.write(pref, value)
    }

    fn write(&self, pref: Pref, value: Value) -> CoreResult<()> {
        self.config.set(&["app", pref.key()], value).map_err(|e| CoreError::Io(e.to_string()))?;
        self.notifier.changed(pref.area());
        Ok(())
    }

    /// Asks the system to start the launcher at login, or not, and keeps why it refused.
    async fn start_at_login(&self, on: bool) -> CoreResult<()> {
        let session = self.ports.login_session.clone();
        if !session.can_start_at_login() {
            return Err(CoreError::Invalid("this build can't start when you log in".into()));
        }
        let done = tokio::task::spawn_blocking(move || session.start_at_login(on))
            .await
            .map_err(|e| CoreError::Io(e.to_string()))?;
        *self.login_error() = done.as_ref().err().cloned();
        if let Err(error) = done {
            self.notifier.changed(Area::Settings);
            return Err(CoreError::Invalid(error));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_pref_is_saved_under_its_json_name() {
        for pref in Pref::ALL {
            assert_eq!(serde_json::to_value(pref).unwrap(), Value::from(pref.key()), "{pref:?}");
            assert_eq!(serde_json::from_value::<Pref>(Value::from(pref.key())).unwrap(), pref);
        }
    }

    #[test]
    fn an_unset_pref_reads_as_its_default() {
        let defaults = [
            (Pref::Sounds, Value::Bool(true)),
            (Pref::Fullscreen, Value::Bool(true)),
            (Pref::ReturnOnExit, Value::Bool(true)),
            (Pref::Scale, Value::from(1.0)),
            (Pref::StartAtLogin, Value::Bool(false)),
            (Pref::AutoUpdate, Value::Bool(true)),
            (Pref::Monitor, Value::Null),
            (Pref::Welcomed, Value::Bool(false)),
            (Pref::VpnOnly, Value::Bool(false)),
            (Pref::Vpn, Value::from("")),
            (Pref::Theme, Value::Null),
            (Pref::PcNoticed, Value::Null),
            (Pref::UpdateRepo, Value::from("mohamed-rekiba/nexus")),
        ];
        assert_eq!(defaults.len(), Pref::ALL.len(), "every pref has a row");
        for (pref, default) in defaults {
            assert_eq!(pref.read(&Value::Null), default, "{pref:?}");
        }
    }

    #[test]
    fn a_saved_value_of_the_wrong_type_reads_as_the_default() {
        assert_eq!(Pref::Sounds.read(&Value::from("off")), Value::Bool(true));
        assert_eq!(Pref::Scale.read(&Value::Bool(true)), Value::from(1.0));
        assert_eq!(Pref::Vpn.read(&Value::from(3)), Value::from(""));
        assert_eq!(Pref::Monitor.read(&Value::from(3)), Value::Null);
        assert_eq!(Pref::Sounds.read(&Value::Bool(false)), Value::Bool(false), "a right one stays");
    }

    #[test]
    fn a_size_goes_to_the_nearest_end_of_its_range() {
        let saved = |size: f64| Pref::Scale.check(Value::from(size)).unwrap();
        let read = |size: f64| Pref::Scale.read(&Value::from(size)).as_f64().unwrap() as f32;
        let rows: [(f64, f64, f32); 5] =
            [(1.2, 1.2, 1.2), (3.0, 1.4, 1.4), (0.1, 0.8, 0.8), (1.0, 1.0, 1.0), (-5.0, 0.8, 0.8)];
        for (size, kept, shown) in rows {
            assert_eq!(saved(size), Value::from(kept), "{size} is saved as {kept}");
            assert_eq!(read(size), shown, "{size} reads as {shown}");
        }
    }

    #[test]
    fn a_size_is_saved_with_two_decimals() {
        assert_eq!(Pref::Scale.check(Value::from(1.2f32)).unwrap(), Value::from(1.2));
        assert_eq!(Pref::Scale.check(Value::from(1.234)).unwrap(), Value::from(1.23));
    }

    #[test]
    fn a_value_of_the_wrong_type_is_refused() {
        // A size that isn't a number, such as NaN, is null in JSON.
        assert!(Pref::Scale.check(Value::from(f32::NAN)).is_err());
        assert!(Pref::Scale.check(Value::from("big")).is_err());
        assert!(Pref::Sounds.check(Value::from(1)).is_err());
        assert!(Pref::Vpn.check(Value::Null).is_err());
        assert!(Pref::Monitor.check(Value::Bool(true)).is_err());
    }

    #[test]
    fn empty_text_is_no_name() {
        assert_eq!(Pref::Monitor.check(Value::from("")).unwrap(), Value::Null);
        assert_eq!(Pref::Monitor.check(Value::from("DP-1")).unwrap(), Value::from("DP-1"));
        assert_eq!(Pref::Vpn.check(Value::from("")).unwrap(), Value::from(""), "an empty VPN is a choice");
    }

    #[test]
    fn the_player_sets_only_their_own_prefs() {
        let by_core: Vec<Pref> = Pref::ALL.into_iter().filter(|p| !p.spec().by_player).collect();
        assert_eq!(by_core, [Pref::Theme, Pref::PcNoticed, Pref::UpdateRepo]);
    }
}
