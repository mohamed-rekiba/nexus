//! The host services the kit uses: the narrow parts of a [`PluginContext`]. The kit never
//! takes the whole context, so its tests run on in-memory doubles of these alone.

use std::sync::Arc;

use nexus_api::{ConfigStore, Folders, HostInfo, HttpClient, Logger, Platform, PluginContext, Programs, Storage};

#[derive(Clone)]
pub struct Host {
    pub storage: Arc<dyn Storage>,
    pub folders: Arc<dyn Folders>,
    pub programs: Arc<dyn Programs>,
    pub http: Arc<dyn HttpClient>,
    pub config: Arc<dyn ConfigStore>,
    pub log: Logger,
    /// The system the app runs on, from [`HostInfo`].
    pub platform: Option<Platform>,
}

impl Host {
    pub fn of(ctx: &PluginContext) -> Self {
        Self {
            storage: ctx.storage().clone(),
            folders: ctx.folders().clone(),
            programs: ctx.programs().clone(),
            http: ctx.http().clone(),
            config: ctx.config().clone(),
            log: ctx.log().clone(),
            platform: ctx.services().get::<HostInfo>().and_then(|h| h.platform),
        }
    }
}
