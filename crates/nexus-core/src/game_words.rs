//! The words on a game's page: the rules that choose between what the game says about itself,
//! what a store's catalog says, and what the core can say when both are silent.
//!
//! The functions are pure. [`crate::view`] reads the library, calls them, and copies the answer
//! into the view model.

use nexus_api::{GameDetails, GameMetadata, PluginId};

/// What the page says about a game, in the words that won.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct GameWords {
    pub description: String,
    pub developer: String,
    pub genres: Vec<String>,
}

/// Where a game with no words of its own comes from, for the last line of [`home_line`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Origin<'a> {
    /// A Catalog offer, named by the emulator's plugin.
    Catalog { emulator: &'a str },
    /// An installed game, played with this program.
    Local { program: &'a str },
}

/// The first text that `field` gives from a layer of `metas` where it isn't empty.
pub(crate) fn first_text(metas: &[(PluginId, GameMetadata)], field: fn(&GameMetadata) -> &String) -> String {
    metas.iter().map(|(_, m)| field(m)).find(|t| !t.is_empty()).cloned().unwrap_or_default()
}

/// The game's own words first; a layer of `metas` (a store's catalog, then a release's store
/// data) fills in, field by field, what they leave out.
pub(crate) fn words(details: &GameDetails, metas: &[(PluginId, GameMetadata)]) -> GameWords {
    let description = if details.description.is_empty() {
        first_text(metas, |m| &m.description)
    } else {
        details.description.clone()
    };
    let developer = details
        .developer
        .clone()
        .or_else(|| Some(first_text(metas, |m| &m.publisher)).filter(|p| !p.is_empty()))
        .unwrap_or_default();
    let genres = if details.genres.is_empty() {
        metas.iter().map(|(_, m)| &m.genres).find(|g| !g.is_empty()).cloned().unwrap_or_default()
    } else {
        details.genres.clone()
    };
    GameWords { description, developer, genres }
}

/// Home's line for a game, as the old launcher said it: the store's tagline, else the game's own
/// description, else its facts on one line (when there are at least two), else where it runs.
pub(crate) fn home_line(
    details: &GameDetails,
    metas: &[(PluginId, GameMetadata)],
    tagline: Option<&str>,
    origin: Origin,
) -> String {
    if let Some(tagline) = tagline.filter(|t| !t.is_empty()) {
        return tagline.to_owned();
    }
    if !details.description.is_empty() {
        return details.description.clone();
    }
    let words = words(details, metas);
    let version = details.facts.iter().find(|f| f.id == "version").map(|f| f.value.as_str()).unwrap_or_default();
    let facts: Vec<String> = [
        details.summary.title.clone(),
        words.genres.join(", "),
        Some(&words.developer).filter(|d| !d.is_empty()).map(|d| format!("Developer: {d}")).unwrap_or_default(),
        Some(version).filter(|v| !v.is_empty()).map(|v| format!("Version: {v}")).unwrap_or_default(),
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect();
    if facts.len() > 1 {
        return facts.join(" · ");
    }
    match origin {
        Origin::Catalog { emulator } => format!("In the catalog for {emulator}."),
        Origin::Local { program } => format!("Installed locally. Launches with the {program} emulator."),
    }
}

#[cfg(test)]
mod tests {
    use nexus_api::{GameFact, GameSummary};

    use super::*;

    fn details(title: &str) -> GameDetails {
        GameDetails {
            summary: GameSummary {
                id: "1".into(),
                title: title.into(),
                platform: "PS5".into(),
                cover: None,
                tags: Vec::new(),
                release_year: None,
                title_id: None,
                content_id: None,
            },
            description: String::new(),
            developer: None,
            genres: Vec::new(),
            size_bytes: None,
            background: None,
            facts: Vec::new(),
            folder: None,
            also: Vec::new(),
            uninstall_link: None,
        }
    }

    fn layer(set: impl FnOnce(&mut GameMetadata)) -> (PluginId, GameMetadata) {
        let mut meta = GameMetadata::default();
        set(&mut meta);
        (PluginId::new("store").unwrap(), meta)
    }

    #[test]
    fn the_game_s_own_words_win_and_a_layer_fills_what_they_leave_out() {
        let mut own = details("Astro");
        own.description = "Its own".into();
        let metas = [layer(|m| {
            m.description = "The store's".into();
            m.publisher = "Studio".into();
            m.genres = vec!["Platformer".into()];
        })];
        let said = words(&own, &metas);
        assert_eq!(said.description, "Its own");
        assert_eq!(said.developer, "Studio", "the store's publisher fills the developer");
        assert_eq!(said.genres, ["Platformer"]);

        own.developer = Some("Own Dev".into());
        own.genres = vec!["Action".into()];
        let said = words(&own, &metas);
        assert_eq!((said.developer.as_str(), said.genres.as_slice()), ("Own Dev", ["Action".to_owned()].as_slice()));
    }

    #[test]
    fn an_empty_layer_falls_through_to_the_next_and_no_layer_leaves_the_words_empty() {
        let metas = [layer(|_| {}), layer(|m| m.description = "From the release".into())];
        assert_eq!(words(&details("A"), &metas).description, "From the release");
        assert_eq!(first_text(&metas, |m| &m.store_url), "");
        assert_eq!(words(&details("A"), &[]), GameWords::default());
    }

    #[test]
    fn the_home_line_is_the_tagline_then_the_description_then_the_facts_then_the_origin() {
        let mut game = details("Astro");
        let origin = Origin::Local { program: "KytyPS5" };
        assert_eq!(home_line(&game, &[], None, origin), "Installed locally. Launches with the KytyPS5 emulator.");
        assert_eq!(home_line(&game, &[], None, Origin::Catalog { emulator: "shadPS4" }), "In the catalog for shadPS4.");

        // One fact is no line: the title alone says nothing.
        game.genres = vec!["Action".into(), "RPG".into()];
        game.developer = Some("Studio".into());
        game.facts = vec![GameFact::new("version", "Version", "1.02")];
        assert_eq!(home_line(&game, &[], None, origin), "Astro · Action, RPG · Developer: Studio · Version: 1.02");

        game.description = "Its own words".into();
        assert_eq!(home_line(&game, &[], None, origin), "Its own words");
        assert_eq!(home_line(&game, &[], Some("Store tagline"), origin), "Store tagline");
        assert_eq!(home_line(&game, &[], Some(""), origin), "Its own words", "an empty tagline is none");
    }

    #[test]
    fn a_description_from_a_store_is_not_the_game_s_own_for_the_home_line() {
        // The line falls back to the facts, not to the store's description: only the game's own
        // words count as its own.
        let game = details("Astro");
        let metas = [layer(|m| m.description = "Store text".into())];
        let said = words(&game, &metas);
        assert_eq!(said.description, "Store text");
        assert_eq!(
            home_line(&game, &metas, None, Origin::Local { program: "KytyPS5" }),
            "Installed locally. Launches with the KytyPS5 emulator."
        );
    }
}
