use serde::{Deserialize, Serialize};

/// Where an item goes among its peers (slot items, middleware).
///
/// `before` and `after` name peer ids and win over `priority`. Between unrelated items, the
/// higher `priority` goes first, then the id decides. Load order never matters.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Order {
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub before: Vec<String>,
    #[serde(default)]
    pub after: Vec<String>,
}

impl Order {
    pub fn priority(priority: i32) -> Self {
        Self { priority, ..Self::default() }
    }

    pub fn before(mut self, id: impl Into<String>) -> Self {
        self.before.push(id.into());
        self
    }

    pub fn after(mut self, id: impl Into<String>) -> Self {
        self.after.push(id.into());
        self
    }
}
