//! Art the player is likely to see next, fetched before they get there: the background and
//! logo of the games beside the selection, and the selected game's screenshots once the
//! selection rests. The fetch is the one the theme host uses (`Core::image_bytes`), so the art
//! is in the image cache when the theme asks for it. Each select stops the last one's
//! fetches, so a fast scroll fetches nothing.

use std::time::Duration;

use nexus_api::GameKey;
use nexus_ui_api::ImageRef;

use crate::Core;

/// How long a selection rests before the art of the games beside it is fetched.
const NEIGHBOURS_AFTER: Duration = Duration::from_millis(150);
/// How long a selection rests before its screenshots are fetched.
const SCREENSHOTS_AFTER: Duration = Duration::from_millis(500);
/// The most games beside the selection whose art is fetched.
const MOST_NEIGHBOURS: usize = 6;

/// Fetches the art around `game`, the game just selected, and stops the fetch for the last
/// selection.
pub(crate) fn prefetch_around(core: &Core, game: &GameKey) {
    let (this, game) = (core.clone(), game.clone());
    let task = tokio::spawn(async move { prefetch(&this, game).await });
    if let Some(last) = core.library().prefetching(task.abort_handle()) {
        last.abort();
    }
}

async fn prefetch(core: &Core, game: GameKey) {
    let clock = core.ports().clock.clone();
    let rested = clock.now() + SCREENSHOTS_AFTER;
    clock.sleep(NEIGHBOURS_AFTER).await;
    let art: Vec<ImageRef> = {
        let snapshot = core.snapshot();
        snapshot
            .neighbours(&game, MOST_NEIGHBOURS)
            .iter()
            .flat_map(|key| {
                let art = snapshot.library.art(key);
                [art.background, art.logo]
            })
            .flatten()
            .map(|(plugin, image)| crate::library::image_ref(&plugin, &image))
            .collect()
    };
    for image in &art {
        fetch_ahead(core, image).await;
    }
    clock.sleep_until(rested).await;
    let shots = core.library().snapshot().art(&game).screenshots;
    let shots: Vec<ImageRef> = shots.iter().map(|(plugin, image)| crate::library::image_ref(plugin, image)).collect();
    for image in &shots {
        fetch_ahead(core, image).await;
    }
}

/// Fetches a web image into the image cache, once a run. Other images are local.
async fn fetch_ahead(core: &Core, image: &ImageRef) {
    let ImageRef::Url { url, .. } = image else { return };
    if !core.library().first_fetch(url) {
        return;
    }
    // A task of its own, which the next select doesn't stop: a stop while the image is
    // being written would leave half a file in the cache.
    let core = core.clone();
    let image = image.clone();
    let fetched = tokio::spawn(async move { core.image_bytes(&image).await }).await;
    if let Ok(Err(error)) = fetched {
        tracing::debug!(%url, %error, "could not fetch art ahead");
    }
}
