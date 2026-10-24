use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A path inside a folder the host owns (a plugin's data folder, a theme folder).
///
/// It is always relative, uses `/`, and has no `.`, `..` or empty parts, so joining it to a
/// base folder can never leave that folder. The empty path is the base folder itself.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RelPath(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("path {0:?} must be relative, use '/', and have no '.', '..' or empty parts")]
pub struct InvalidPath(pub String);

impl RelPath {
    pub fn new(path: impl Into<String>) -> Result<Self, InvalidPath> {
        let path = path.into();
        if path.is_empty() {
            return Ok(Self(path));
        }
        let bad_part =
            |part: &str| part.is_empty() || part == "." || part == ".." || part.contains('\\') || part.contains(':');
        if path.starts_with('/') || path.split('/').any(bad_part) {
            return Err(InvalidPath(path));
        }
        Ok(Self(path))
    }

    /// The base folder itself.
    pub fn root() -> Self {
        Self(String::new())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// Appends `child`, which must itself be a valid relative path.
    pub fn join(&self, child: &str) -> Result<Self, InvalidPath> {
        let child = Self::new(child)?;
        Ok(match (self.is_root(), child.is_root()) {
            (true, _) => child,
            (false, true) => self.clone(),
            (false, false) => Self(format!("{}/{}", self.0, child.0)),
        })
    }

    /// Joins this path to `base`. The result is always inside `base`.
    pub fn under(&self, base: &Path) -> PathBuf {
        self.0.split('/').filter(|p| !p.is_empty()).fold(base.to_path_buf(), |acc, part| acc.join(part))
    }

    pub fn file_name(&self) -> Option<&str> {
        self.0.rsplit('/').next().filter(|s| !s.is_empty())
    }

    pub fn extension(&self) -> Option<&str> {
        self.file_name()?.rsplit_once('.').map(|(_, ext)| ext)
    }
}

impl TryFrom<String> for RelPath {
    type Error = InvalidPath;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for RelPath {
    type Error = InvalidPath;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<RelPath> for String {
    fn from(path: RelPath) -> Self {
        path.0
    }
}

impl fmt::Display for RelPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_every_way_out_of_the_base_folder() {
        for bad in ["/etc/passwd", "../x", "a/../../x", "a//b", "./a", "a/.", "C:/x", "a\\..\\b"] {
            assert!(RelPath::new(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn joins_stay_under_the_base() {
        let base = Path::new("/data/emu");
        let p = RelPath::new("builds/1.0/kyty").unwrap();
        assert_eq!(p.under(base), PathBuf::from("/data/emu/builds/1.0/kyty"));
        assert_eq!(RelPath::root().under(base), base);
        assert_eq!(RelPath::root().join("a").unwrap().as_str(), "a");
        assert!(p.join("../x").is_err());
        assert_eq!(p.extension(), None);
        assert_eq!(RelPath::new("a/b.slint").unwrap().extension(), Some("slint"));
    }
}
