//! In-memory doubles of the host facades, for the kit's own tests.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nexus_api::serde_json::Value;
use nexus_api::{
    ConfigScope, ConfigStore, DirEntry, FolderRoot, Folders, HostPath, HttpClient, HttpRequest, HttpResponse, Location,
    LogLevel, LogSink, Logger, PluginError, PluginResult, Programs, RelPath, Storage, async_trait,
};

use crate::host::Host;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// A data folder in memory: files by path, folders implied by the files in them. Every call
/// yields once first, so calls running at the same time interleave as on a real disk.
#[derive(Default)]
pub struct MemStorage {
    files: Mutex<BTreeMap<String, Vec<u8>>>,
    executable: Mutex<Vec<String>>,
}

fn inside(path: &str, dir: &str) -> bool {
    dir.is_empty() || path == dir || path.starts_with(&format!("{dir}/"))
}

impl MemStorage {
    pub fn put(&self, path: &str, data: &[u8]) {
        lock(&self.files).insert(path.to_owned(), data.to_vec());
    }

    pub fn file(&self, path: &str) -> Option<Vec<u8>> {
        lock(&self.files).get(path).cloned()
    }

    /// Every file path, sorted.
    pub fn paths(&self) -> Vec<String> {
        lock(&self.files).keys().cloned().collect()
    }

    pub fn is_executable(&self, path: &str) -> bool {
        lock(&self.executable).iter().any(|p| p == path)
    }
}

#[async_trait]
impl Storage for MemStorage {
    async fn read(&self, path: &RelPath) -> PluginResult<Vec<u8>> {
        tokio::task::yield_now().await;
        self.file(path.as_str()).ok_or_else(|| PluginError::NotFound(path.to_string()))
    }

    async fn write(&self, path: &RelPath, data: &[u8]) -> PluginResult<()> {
        tokio::task::yield_now().await;
        self.put(path.as_str(), data);
        Ok(())
    }

    async fn write_at(&self, path: &RelPath, offset: u64, data: &[u8]) -> PluginResult<()> {
        tokio::task::yield_now().await;
        let mut files = lock(&self.files);
        let file = files.entry(path.as_str().to_owned()).or_default();
        let start = usize::try_from(offset).map_err(|e| PluginError::InvalidInput(e.to_string()))?;
        if file.len() < start + data.len() {
            file.resize(start + data.len(), 0);
        }
        file[start..start + data.len()].copy_from_slice(data);
        Ok(())
    }

    async fn exists(&self, path: &RelPath) -> PluginResult<bool> {
        tokio::task::yield_now().await;
        Ok(lock(&self.files).keys().any(|p| inside(p, path.as_str())))
    }

    async fn list(&self, dir: &RelPath) -> PluginResult<Vec<String>> {
        tokio::task::yield_now().await;
        let prefix = if dir.is_root() { String::new() } else { format!("{dir}/") };
        let names: std::collections::BTreeSet<String> = lock(&self.files)
            .keys()
            .filter_map(|p| p.strip_prefix(&prefix))
            .filter_map(|rest| rest.split('/').next())
            .map(str::to_owned)
            .collect();
        Ok(names.into_iter().collect())
    }

    async fn remove(&self, path: &RelPath) -> PluginResult<()> {
        tokio::task::yield_now().await;
        lock(&self.files).retain(|p, _| !inside(p, path.as_str()));
        Ok(())
    }

    async fn rename(&self, from: &RelPath, to: &RelPath) -> PluginResult<()> {
        tokio::task::yield_now().await;
        let mut files = lock(&self.files);
        let moved: Vec<String> = files.keys().filter(|p| inside(p, from.as_str())).cloned().collect();
        if moved.is_empty() {
            return Err(PluginError::NotFound(from.to_string()));
        }
        for old in moved {
            let data = files.remove(&old).unwrap_or_default();
            files.insert(format!("{to}{}", &old[from.as_str().len()..]), data);
        }
        Ok(())
    }

    async fn set_executable(&self, path: &RelPath) -> PluginResult<()> {
        lock(&self.executable).push(path.to_string());
        Ok(())
    }
}

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
