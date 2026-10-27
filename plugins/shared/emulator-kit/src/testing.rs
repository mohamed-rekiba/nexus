//! In-memory doubles of the host facades, for the kit's own tests.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nexus_api::serde_json::Value;
use nexus_api::{
    ConfigScope, ConfigStore, DirEntry, FolderRoot, Folders, HostPath, HttpClient, HttpRequest, HttpResponse, Location,
    LogLevel, LogSink, Logger, PluginError, PluginResult, Programs, async_trait,
};

use crate::host::Host;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

pub use nexus_testkit::MemStorage;

/// The plugin's stored settings, in memory.
#[derive(Default)]
pub struct MemConfig {
    values: Mutex<HashMap<String, Value>>,
}

impl MemConfig {
    pub fn emulator(&self) -> Value {
        lock(&self.values).get("emulator").cloned().unwrap_or(Value::Null)
    }
}

fn scope_key(scope: &ConfigScope) -> String {
    match scope {
        ConfigScope::Emulator => "emulator".into(),
        ConfigScope::Game(game) => format!("game:{game}"),
    }
}

#[async_trait]
impl ConfigStore for MemConfig {
    async fn get(&self, scope: &ConfigScope) -> PluginResult<Value> {
        Ok(lock(&self.values).get(&scope_key(scope)).cloned().unwrap_or(Value::Null))
    }

    async fn set(&self, scope: &ConfigScope, value: Value) -> PluginResult<()> {
        lock(&self.values).insert(scope_key(scope), value);
        Ok(())
    }
}

/// A program's real path, and what it prints when it runs.
type FakeProgram = (String, Option<Vec<u8>>);

/// Programs by where they are: each with its real path and what it prints for any arguments.
/// A program without output fails when run.
#[derive(Default)]
pub struct FakePrograms {
    programs: Mutex<HashMap<String, FakeProgram>>,
}

impl FakePrograms {
    pub fn add(&self, at: &Location, real: &str, output: Option<&str>) {
        lock(&self.programs).insert(at.to_string(), (real.to_owned(), output.map(|o| o.as_bytes().to_vec())));
    }
}

#[async_trait]
impl Programs for FakePrograms {
    async fn find(&self, program: &Location) -> PluginResult<Option<String>> {
        Ok(lock(&self.programs).get(&program.to_string()).map(|(real, _)| real.clone()))
    }

    async fn output(&self, program: &Location, _args: &[String], _timeout: Duration) -> PluginResult<Vec<u8>> {
        match lock(&self.programs).get(&program.to_string()) {
            Some((_, Some(out))) => Ok(out.clone()),
            _ => Err(PluginError::Other(format!("{program} does not run"))),
        }
    }
}

/// Web pages by URL. Keeps every request it gets.
#[derive(Default)]
pub struct FakeHttp {
    pages: Mutex<HashMap<String, Vec<u8>>>,
    requests: Mutex<Vec<HttpRequest>>,
}

impl FakeHttp {
    pub fn page(&self, url: &str, body: &str) {
        lock(&self.pages).insert(url.to_owned(), body.as_bytes().to_vec());
    }

    pub fn requests(&self) -> Vec<HttpRequest> {
        lock(&self.requests).clone()
    }
}

#[async_trait]
impl HttpClient for FakeHttp {
    async fn send(&self, request: HttpRequest) -> PluginResult<HttpResponse> {
        let body = lock(&self.pages).get(&request.url).cloned();
        lock(&self.requests).push(request);
        Ok(match body {
            Some(body) => HttpResponse { status: 200, headers: Vec::new(), body },
            None => HttpResponse { status: 404, headers: Vec::new(), body: Vec::new() },
        })
    }
}

/// Host folders the player has not mapped.
pub struct NoFolders;

#[async_trait]
impl Folders for NoFolders {
    async fn roots(&self, _folder: &str) -> PluginResult<Vec<FolderRoot>> {
        Ok(Vec::new())
    }
    async fn list(&self, _dir: &HostPath) -> PluginResult<Vec<DirEntry>> {
        Ok(Vec::new())
    }
    async fn size(&self, _dir: &HostPath) -> PluginResult<u64> {
        Ok(0)
    }
    async fn exists(&self, _path: &HostPath) -> PluginResult<bool> {
        Ok(false)
    }
    async fn read_range(&self, path: &HostPath, _offset: u64, _len: u64) -> PluginResult<Vec<u8>> {
        Err(PluginError::NotFound(path.path.to_string()))
    }
    async fn write(&self, path: &HostPath, _data: &[u8]) -> PluginResult<()> {
        Err(PluginError::PermissionDenied(path.path.to_string()))
    }
    async fn write_at(&self, path: &HostPath, _offset: u64, _data: &[u8]) -> PluginResult<()> {
        Err(PluginError::PermissionDenied(path.path.to_string()))
    }
    async fn rename(&self, from: &HostPath, _to: &HostPath) -> PluginResult<()> {
        Err(PluginError::PermissionDenied(from.path.to_string()))
    }
    async fn remove(&self, _path: &HostPath) -> PluginResult<()> {
        Ok(())
    }
    async fn set_executable(&self, path: &HostPath) -> PluginResult<()> {
        Err(PluginError::PermissionDenied(path.path.to_string()))
    }
}

/// Log lines, kept.
#[derive(Default)]
pub struct Logs(Mutex<Vec<(LogLevel, String)>>);

impl LogSink for Logs {
    fn log(&self, level: LogLevel, message: &str) {
        lock(&self.0).push((level, message.to_owned()));
    }
}

/// A host of doubles, with handles on each.
#[derive(Default)]
pub struct TestHost {
    pub storage: Arc<MemStorage>,
    pub config: Arc<MemConfig>,
    pub programs: Arc<FakePrograms>,
    pub http: Arc<FakeHttp>,
    pub logs: Arc<Logs>,
}

impl TestHost {
    /// The facades the kit takes, over these doubles.
    pub fn host(&self) -> Host {
        Host {
            storage: self.storage.clone(),
            folders: Arc::new(NoFolders),
            programs: self.programs.clone(),
            http: self.http.clone(),
            config: self.config.clone(),
            log: Logger::new(self.logs.clone()),
            platform: Some(nexus_api::Platform::Linux),
        }
    }
}
