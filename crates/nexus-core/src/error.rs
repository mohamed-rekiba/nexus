use nexus_api::{Capability, JobId, PluginError, PluginId, SessionId};

/// Why the core refused to load a plugin. Boot carries on without it.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum LoadError {
    #[error("bad manifest in {source_name}: {message}")]
    Manifest { source_name: String, message: String },
    #[error("needs host API {required}, this host has {actual}")]
    HostApi { required: String, actual: String },
    #[error("doesn't support this platform (supports {supported:?})")]
    Platform { supported: Vec<nexus_api::Platform> },
    #[error("another plugin already has this id")]
    Duplicate,
    #[error("needs {dependency}, which isn't installed")]
    MissingDependency { dependency: PluginId },
    #[error("needs {dependency} {required}, found {found}")]
    DependencyVersion { dependency: PluginId, required: String, found: String },
    #[error("needs {dependency}, which failed to load")]
    DependencyRejected { dependency: PluginId },
    #[error("dependency cycle between {members:?}")]
    Cycle { members: Vec<PluginId> },
    #[error("safe mode loads built-in plugins only")]
    SafeMode,
    #[error("{0}")]
    Invalid(String),
    #[error("no loader for this plugin: {0}")]
    Loader(String),
    #[error("init failed: {0}")]
    Init(PluginError),
    #[error("declared capabilities {declared:?}, but init provided {provided:?}")]
    CapabilityMismatch { declared: Vec<Capability>, provided: Vec<Capability> },
}

/// An error from the core's public API.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CoreError {
    #[error("no plugin {0}")]
    UnknownPlugin(PluginId),
    #[error("plugin {plugin} is not available: {reason}")]
    PluginUnavailable { plugin: PluginId, reason: String },
    #[error("plugin {plugin} does not provide {capability:?}")]
    MissingCapability { plugin: PluginId, capability: Capability },
    #[error("plugin {plugin}: {error}")]
    Plugin { plugin: PluginId, error: PluginError },
    #[error("plugin {plugin} is not allowed to {action}")]
    PermissionDenied { plugin: PluginId, action: String },
    #[error("no game {0}")]
    UnknownGame(String),
    #[error("no download {0}")]
    UnknownJob(JobId),
    #[error("download {0} was cancelled")]
    Cancelled(JobId),
    #[error("no running game {0}")]
    UnknownSession(SessionId),
    #[error("download failed: {0}")]
    Download(String),
    #[error("{0}")]
    Invalid(String),
    #[error("storage error: {0}")]
    Io(String),
}

pub type CoreResult<T> = Result<T, CoreError>;

/// A capability the plugin must provide for this call, or [`CoreError::MissingCapability`].
pub(crate) fn require<T: ?Sized>(
    plugin: &PluginId,
    capability: Capability,
    provided: Option<std::sync::Arc<T>>,
) -> CoreResult<std::sync::Arc<T>> {
    provided.ok_or_else(|| CoreError::MissingCapability { plugin: plugin.clone(), capability })
}
