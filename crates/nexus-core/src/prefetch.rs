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

impl Core {
    /// Fetches the art around `game`, the game just selected, and stops the fetch for the last
    /// selection.
    pub(crate) fn prefetch_around(&self, game: &GameKey) {
        let core = self.clone();
        let game = game.clone();
        let task = tokio::spawn(async move { core.prefetch(game).await });
        if let Some(last) = self.state().prefetch.replace(task.abort_handle()) {
            last.abort();
        }
    }

    async fn prefetch(&self, game: GameKey) {
        let clock = self.kernel().clock().clone();
        let rested = clock.now() + SCREENSHOTS_AFTER;
        clock.sleep(NEIGHBOURS_AFTER).await;
        let art: Vec<ImageRef> = {
            let state = self.state();
            crate::view::neighbours(&self.inner.kernel, &state, &game, &state.grid_columns, MOST_NEIGHBOURS)
                .iter()
                .flat_map(|key| {
                    let art = crate::view::hub_art(&state, key);
                    [art.background, art.logo]
                })
                .flatten()
                .collect()
        };
        for image in &art {
            self.fetch_ahead(image).await;
        }
        clock.sleep_until(rested).await;
        let shots = crate::view::hub_art(&self.state(), &game).screenshots;
        for image in &shots {
            self.fetch_ahead(image).await;
        }
    }

    /// Fetches a web image into the image cache, once a run. Other images are local.
    async fn fetch_ahead(&self, image: &ImageRef) {
        let ImageRef::Url { url, .. } = image else { return };
        if !self.state().prefetched.insert(url.clone()) {
            return;
        }
        // A task of its own, which the next select doesn't stop: a stop while the image is
        // being written would leave half a file in the cache.
        let core = self.clone();
        let image = image.clone();
        let fetched = tokio::spawn(async move { core.image_bytes(&image).await }).await;
        if let Ok(Err(error)) = fetched {
            tracing::debug!(%url, %error, "could not fetch art ahead");
        }
    }
}
