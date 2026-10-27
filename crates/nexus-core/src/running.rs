//! The core's own calls to the system's opener, such as opening a store's link, off the async
//! threads. The running games' window and process calls are the sessions subsystem's
//! ([`crate::sessions`]).

use crate::Core;

/// Opens `link` with the system's opener, off the async threads. The caller checked it against
/// the plugin's `permissions.links`.
pub(crate) async fn open_link(core: &Core, link: String) -> Result<(), String> {
    let opener = core.kernel().opener().clone();
    tokio::task::spawn_blocking(move || opener.open_link(&link)).await.expect("the opener doesn't panic")
}
