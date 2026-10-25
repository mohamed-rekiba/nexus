use std::any::Any;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::time::Duration;

use futures::FutureExt;
use nexus_api::{PluginError, PluginId, PluginResult};
use tracing::Instrument;

use crate::Clock;

/// Runs one call into a plugin inside a `plugin_call` span tagged with `plugin_id`, with a
/// timeout and (for in-process plugins) a panic guard.
///
/// The timeout runs on `clock`, and fires at the plugin's next `.await`. A plugin that blocks
/// its thread without awaiting holds that worker thread until it returns.
pub(crate) async fn guarded<T>(
    clock: &dyn Clock,
    plugin: &PluginId,
    method: &'static str,
    timeout: Duration,
    call: impl Future<Output = PluginResult<T>>,
) -> PluginResult<T> {
    let span = tracing::info_span!("plugin_call", plugin_id = %plugin, method);
    let call = AssertUnwindSafe(call).catch_unwind();
    match clock.timeout(timeout, call).instrument(span.clone()).await {
        Some(Ok(result)) => result,
        Some(Err(panic)) => {
            let message = panic_message(panic.as_ref());
            span.in_scope(|| tracing::error!(%message, "plugin panicked"));
            Err(PluginError::Panicked(message))
        }
        None => {
            span.in_scope(|| tracing::error!(?timeout, "plugin call timed out"));
            Err(PluginError::Timeout(timeout.as_millis().try_into().unwrap_or(u64::MAX)))
        }
    }
}

fn panic_message(panic: &(dyn Any + Send)) -> String {
    if let Some(s) = panic.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = panic.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_owned()
    }
}
