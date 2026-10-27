//! The downloads' saved records: each download under `app.torrents.<info hash>`, and the
//! "Keep sharing finished downloads" setting, `app.seed_downloads` (on by default).

use std::sync::Arc;

use serde_json::Value;

use super::TorrentJob;
use crate::config::ConfigService;

/// Where downloads are saved in the config.
const TORRENTS: [&str; 2] = ["app", "torrents"];
const SEED_DOWNLOADS: [&str; 2] = ["app", "seed_downloads"];

/// The subsystem's part of the config.
pub(crate) struct Records {
    config: Arc<ConfigService>,
}

impl Records {
    pub fn new(config: Arc<ConfigService>) -> Self {
        Self { config }
    }

    /// Every saved download. A record that doesn't read is left out.
    pub fn load(&self) -> Vec<TorrentJob> {
        let records = self.config.get(&TORRENTS);
        let Some(map) = records.as_object() else { return Vec::new() };
        map.values().filter_map(|v| serde_json::from_value::<TorrentJob>(v.clone()).ok()).collect()
    }

    pub fn save(&self, job: &TorrentJob) {
        let value = serde_json::to_value(job).unwrap_or_default();
        if let Err(e) = self.config.set(&[TORRENTS[0], TORRENTS[1], &job.id], value) {
            tracing::warn!(error = %e, "could not save a download");
        }
    }

    pub fn forget(&self, id: &str) {
        if let Err(e) = self.config.remove(&[TORRENTS[0], TORRENTS[1], id]) {
            tracing::warn!(error = %e, "could not forget a download");
        }
    }

    /// "Keep sharing finished downloads".
    pub fn sharing(&self) -> bool {
        self.config.get(&SEED_DOWNLOADS).as_bool().unwrap_or(true)
    }

    pub fn set_sharing(&self, on: bool) -> std::io::Result<()> {
        self.config.set(&SEED_DOWNLOADS, Value::Bool(on))
    }

    /// The config, for the VPN rule's settings, which the subsystem reads and never writes.
    pub fn config(&self) -> &ConfigService {
        &self.config
    }
}
