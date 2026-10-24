use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// The id of a plugin: 1 to 64 lowercase ASCII letters, digits or `-`, starting with a letter.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PluginId(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid id {0:?}: use 1 to 64 lowercase letters, digits or '-', starting with a letter")]
pub struct InvalidId(pub String);

impl PluginId {
    pub fn new(id: impl Into<String>) -> Result<Self, InvalidId> {
        let id = id.into();
        let mut chars = id.chars();
        let starts_with_letter = chars.next().is_some_and(|c| c.is_ascii_lowercase());
        let rest_ok = chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if starts_with_letter && rest_ok && id.len() <= 64 { Ok(Self(id)) } else { Err(InvalidId(id)) }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for PluginId {
    type Error = InvalidId;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<PluginId> for String {
    fn from(id: PluginId) -> Self {
        id.0
    }
}

impl FromStr for PluginId {
    type Err = InvalidId;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl fmt::Display for PluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A game across all plugins: the owning plugin plus the plugin's own id for the game.
/// Written as `plugin:local`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct GameKey {
    pub plugin: PluginId,
    pub local: String,
}

impl GameKey {
    pub fn new(plugin: PluginId, local: impl Into<String>) -> Self {
        Self { plugin, local: local.into() }
    }
}

impl FromStr for GameKey {
    type Err = InvalidId;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (plugin, local) = s.split_once(':').ok_or_else(|| InvalidId(s.to_owned()))?;
        if local.is_empty() {
            return Err(InvalidId(s.to_owned()));
        }
        Ok(Self::new(PluginId::new(plugin)?, local))
    }
}

impl TryFrom<String> for GameKey {
    type Error = InvalidId;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<GameKey> for String {
    fn from(key: GameKey) -> Self {
        key.to_string()
    }
}

impl fmt::Display for GameKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.plugin, self.local)
    }
}

/// A download job, numbered by the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct JobId(pub u64);

/// A running game process, numbered by the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SessionId(pub u64);

impl fmt::Display for JobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "job-{}", self.0)
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "session-{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_ids_follow_the_documented_rule() {
        assert!(PluginId::new("emu-kyty").is_ok());
        assert!(PluginId::new("a1").is_ok());
        for bad in ["", "1abc", "Emu", "emu_kyty", "emu native", &"a".repeat(65)] {
            assert!(PluginId::new(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn game_keys_round_trip_through_text() {
        let key: GameKey = "emu-kyty:CUSA00001".parse().unwrap();
        assert_eq!(key.plugin.as_str(), "emu-kyty");
        assert_eq!(key.local, "CUSA00001");
        assert_eq!(key.to_string(), "emu-kyty:CUSA00001");
        assert!("emu-kyty:".parse::<GameKey>().is_err());
        assert!("no-colon".parse::<GameKey>().is_err());
    }
}
