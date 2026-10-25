//! The plugins' own downloads ([`crate::Command::Download`]): plan one with its plugin and queue
//! it with the download manager. Each stays saved under `app.downloads` until it ends, so a
//! crash resumes it.

use nexus_api::{Capability, DownloadRequest, PluginId};
use nexus_ui_api::NoticeKind;

use crate::CoreResult;
use crate::app::{Core, Route};
use crate::changes::Area;
use crate::error::require;

/// A download in progress, saved under `app.downloads` so a crash resumes it.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct DownloadRecord {
    plugin: PluginId,
    request: DownloadRequest,
}

impl DownloadRecord {
    /// The same request for the same plugin is saved once.
    fn key(&self) -> String {
        serde_json::to_string(self).expect("records serialize")
    }
}

/// Plans a download and queues it with the download manager. It stays saved until it
/// ends, so a crash resumes it.
pub(crate) async fn download(core: &Core, plugin: &PluginId, request: DownloadRequest) -> CoreResult<()> {
    let kernel = core.kernel();
    let provided = kernel.provided(plugin).await?;
    let downloader = require(plugin, Capability::Downloader, provided.downloader)?;
    let plan = kernel.guard(plugin, "downloader.plan", downloader.plan(&request)).await?;
    let record = DownloadRecord { plugin: plugin.clone(), request };
    let key = record.key();
    let saved = serde_json::to_value(&record).expect("records serialize");
    if let Err(e) = kernel.config().set(&["app", "downloads", &key], saved) {
        tracing::warn!(error = %e, "could not save download state");
    }
    // The manager removes the record and reports the end as an event.
    let _ = core.inner.downloads.submit(plugin, plan, Some(key));
    core.changed(Area::Downloads);
    Ok(())
}

/// Re-runs downloads that were in progress when the app last stopped. Their part files
/// are still there, so each continues where it stopped.
pub(crate) fn resume_downloads(core: &Core) {
    let pending = core.kernel().config().get(&["app", "downloads"]);
    let Some(pending) = pending.as_object() else { return };
    for (key, saved) in pending.clone() {
        let Ok(record) = serde_json::from_value::<DownloadRecord>(saved) else {
            if let Err(e) = core.kernel().config().remove(&["app", "downloads", &key]) {
                tracing::warn!(error = %e, "could not forget an unreadable download");
            }
            continue;
        };
        tracing::info!(plugin_id = %record.plugin, request = ?record.request, "resuming an interrupted download");
        let core = core.clone();
        tokio::spawn(async move {
            // Saved again under its own key, then removed when it ends.
            if let Err(e) = core.kernel().config().remove(&["app", "downloads", &key]) {
                tracing::warn!(error = %e, "could not save download state");
            }
            if let Err(e) = download(&core, &record.plugin, record.request).await {
                core.notices().opening(
                    NoticeKind::Error,
                    Route::Downloads,
                    format!("Could not resume a download for {}: {e}", record.plugin),
                );
            }
        });
    }
}
