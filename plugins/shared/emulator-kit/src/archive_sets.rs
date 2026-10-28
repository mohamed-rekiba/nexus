//! The archive sets of a download, by name, as the host finds them: what the host unpacks into
//! `unpacked/<folder>/<name>/`. The host's rules are `crates/nexus-core/src/archives.rs`
//! (`part_of`); these follow them, and the tests are the same cases.

/// The folder the host unpacks a download's archive sets into.
pub const UNPACKED: &str = "unpacked";

/// The set a file of a download is a part of, as its folder under [`UNPACKED`] is named: the
/// file's folder and stem, spelled as the file is. `None` for a file that isn't an archive,
/// or one more than one folder down (a game's own data, never unpacked).
pub fn set_name(path: &str) -> Option<String> {
    if path.matches('/').count() > 1 {
        return None;
    }
    let lower = path.to_ascii_lowercase();
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let stem = if let Some(stem) = lower.strip_suffix(".rar") {
        match stem.rsplit_once(".part") {
            Some((name, part)) if digits(part) => name.len(),
            _ => stem.len(),
        }
    } else if let Some((stem, ext)) = lower.rsplit_once('.')
        && let Some(n) = ext.strip_prefix('r')
        && n.len() == 2
        && digits(n)
    {
        stem.len()
    } else if let Some((stem, ext)) = lower.rsplit_once('.')
        && let Some(name) = stem.strip_suffix(".7z")
        && digits(ext)
    {
        name.len()
    } else {
        const SINGLE: [&str; 14] = [
            ".tar.gz", ".tgz", ".tar.xz", ".txz", ".tar.bz2", ".tbz2", ".tbz", ".tar.zst", ".tzst", ".iso", ".cab",
            ".tar", ".7z", ".zip",
        ];
        let ext = SINGLE.into_iter().find(|e| lower.ends_with(e))?;
        lower.len() - ext.len()
    };
    Some(path[..stem].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_part_is_known_by_its_name_as_the_host_knows_it() {
        assert_eq!(set_name("Game.part3.rar").as_deref(), Some("Game"));
        assert_eq!(set_name("game.rar").as_deref(), Some("game"));
        assert_eq!(set_name("game.r00").as_deref(), Some("game"));
        assert_eq!(set_name("a/game.7z.002").as_deref(), Some("a/game"));
        assert_eq!(set_name("game.7z").as_deref(), Some("game"));
        assert_eq!(set_name("game.tar.gz").as_deref(), Some("game"));
        assert_eq!(set_name("game.tgz").as_deref(), Some("game"));
        assert_eq!(set_name("data.tar").as_deref(), Some("data"));
        assert_eq!(set_name("game.tar.xz").as_deref(), Some("game"));
        assert_eq!(set_name("game.tbz2").as_deref(), Some("game"));
        assert_eq!(set_name("game.tar.zst").as_deref(), Some("game"));
        assert_eq!(set_name("disc.iso").as_deref(), Some("disc"));
        assert_eq!(set_name("data1.cab").as_deref(), Some("data1"));
        assert_eq!(set_name("game.zip").as_deref(), Some("game"));
        assert_eq!(set_name("game.pkg"), None);
        assert_eq!(set_name("music.rock"), None);
        assert_eq!(set_name("Game/Data/levels.zip"), None, "two folders down");
    }
}
