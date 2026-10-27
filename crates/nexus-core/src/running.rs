//! The core's own desktop calls, such as opening a store's link, off the async threads. The
//! running games' calls are the sessions subsystem's ([`crate::sessions`]).

use crate::{Core, Desktop, ProcessTree};

/// Runs a desktop or process-list call, which may run a helper program, off the async threads.
pub(crate) async fn blocking<T: Send + 'static>(
    core: &Core,
    call: impl FnOnce(&dyn Desktop, &dyn ProcessTree) -> T + Send + 'static,
) -> T {
    let (desktop, processes) = (core.kernel().desktop().clone(), core.kernel().processes().clone());
    tokio::task::spawn_blocking(move || call(desktop.as_ref(), processes.as_ref()))
        .await
        .expect("desktop and process calls don't panic")
}
