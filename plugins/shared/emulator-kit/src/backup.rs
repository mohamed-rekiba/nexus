//! A plugin's backup items when they are the same on every run: a list the plugin builds once.

use std::sync::Arc;

use nexus_api::{Backup, BackupItem, PluginResult, async_trait};

/// Answers [`Backup::items`] with the same items each time.
pub struct Items(pub Vec<BackupItem>);

impl Items {
    pub fn provided(items: Vec<BackupItem>) -> Arc<dyn Backup> {
        Arc::new(Self(items))
    }
}

#[async_trait]
impl Backup for Items {
    async fn items(&self) -> PluginResult<Vec<BackupItem>> {
        Ok(self.0.clone())
    }
}
