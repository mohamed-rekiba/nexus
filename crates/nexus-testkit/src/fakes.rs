use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::StreamExt;
use nexus_api::{HttpRequest, HttpResponse, PluginError, PluginResult};
use nexus_core::{
    Body, Desktop, Front, HttpTransport, Opened, ProcessExit, ProcessHandle, ProcessSpawner, SpawnCommand,
};
use sha2::{Digest, Sha256};
use tokio::sync::{oneshot, watch};

#[derive(Default)]
struct HttpState {
    files: BTreeMap<String, Vec<u8>>,
    /// URL -> bytes to send before the connection "drops", for the next request only.
    cut_after: BTreeMap<String, usize>,
    no_range: bool,
    /// URL -> bytes to send before waiting for the gate, for the next request only.
    hold_after: BTreeMap<String, (usize, watch::Receiver<bool>)>,
    /// URL -> the gate the next plain request waits for before it answers.
    hold_send: BTreeMap<String, watch::Receiver<bool>>,
    requests: Vec<(String, u64)>,
    /// URL -> where it redirects.
    redirects: BTreeMap<String, String>,
    /// URLs of plain (not download) requests.
    sent: Vec<String>,
}

/// Lets a held download, request, lookup or unpack go on. Dropping it also lets it go on.
pub struct Gate(watch::Sender<bool>);

impl Gate {
    /// A closed gate, and what waits for it: `wait_for(|open| *open)`.
    pub(crate) fn new() -> (Gate, watch::Receiver<bool>) {
        let (tx, rx) = watch::channel(false);
        (Gate(tx), rx)
    }

    pub fn release(&self) {
        let _ = self.0.send(true);
    }
}

impl Drop for Gate {
    fn drop(&mut self) {
        self.release();
    }
}

/// An in-memory web server.
#[derive(Clone, Default)]
pub struct FakeHttp {
    state: Arc<Mutex<HttpState>>,
}

impl FakeHttp {
    fn lock(&self) -> std::sync::MutexGuard<'_, HttpState> {
        self.state.lock().unwrap()
    }

    /// Serves `body` at `url` and returns its SHA-256 in hex.
    pub fn serve(&self, url: &str, body: impl Into<Vec<u8>>) -> String {
        let body = body.into();
        let sha = hex::encode(Sha256::digest(&body));
        self.lock().files.insert(url.to_owned(), body);
        sha
    }

    /// The next download of `url` stops with an error after `bytes` bytes.
    pub fn cut_next_after(&self, url: &str, bytes: usize) {
        self.lock().cut_after.insert(url.to_owned(), bytes);
    }

    /// The next download of `url` sends `bytes` bytes, then waits until the gate is released.
    pub fn hold_after(&self, url: &str, bytes: usize) -> Gate {
        let (tx, rx) = watch::channel(false);
        self.lock().hold_after.insert(url.to_owned(), (bytes, rx));
        Gate(tx)
    }

    /// The next plain request of `url` waits until the gate is released, then answers.
    pub fn hold_next(&self, url: &str) -> Gate {
        let (tx, rx) = watch::channel(false);
        self.lock().hold_send.insert(url.to_owned(), rx);
        Gate(tx)
    }

    /// Answers `from` with a 302 to `to` (absolute or relative).
    pub fn redirect(&self, from: &str, to: &str) {
        self.lock().redirects.insert(from.to_owned(), to.to_owned());
    }

    /// Ignore Range requests and always send the whole file.
    pub fn ignore_ranges(&self) {
        self.lock().no_range = true;
    }

    /// Every plain request so far (not downloads), by URL.
    pub fn sent(&self) -> Vec<String> {
        self.lock().sent.clone()
    }

    /// Every download request so far: URL and starting offset.
    pub fn requests(&self) -> Vec<(String, u64)> {
        self.lock().requests.clone()
    }

    /// Every URL asked for so far that this server doesn't serve, once each. It answers them
    /// with a 404, never with a request to the network.
    pub fn unserved(&self) -> Vec<String> {
        let state = self.lock();
        let asked = state.sent.iter().chain(state.requests.iter().map(|(url, _)| url));
        let mut out: Vec<String> = asked
            .filter(|url| !state.files.contains_key(*url) && !state.redirects.contains_key(*url))
            .cloned()
            .collect();
        out.sort();
        out.dedup();
        out
    }
}

#[async_trait]
impl HttpTransport for FakeHttp {
    async fn send(&self, request: HttpRequest) -> PluginResult<HttpResponse> {
        let held = {
            let mut state = self.lock();
            state.sent.push(request.url.clone());
            state.hold_send.remove(&request.url)
        };
        // Not under the lock: other requests answer while this one waits.
        if let Some(mut gate) = held {
            let _ = gate.wait_for(|open| *open).await;
        }
        let state = self.lock();
        if let Some(to) = state.redirects.get(&request.url) {
            let headers = vec![("Location".to_owned(), to.clone())];
            return Ok(HttpResponse { status: 302, headers, body: Vec::new() });
        }
        match state.files.get(&request.url) {
            Some(body) => Ok(HttpResponse { status: 200, headers: Vec::new(), body: body.clone() }),
            None => Ok(HttpResponse { status: 404, headers: Vec::new(), body: Vec::new() }),
        }
    }

    async fn open(&self, url: &str, offset: u64) -> PluginResult<Opened> {
        let mut state = self.lock();
        state.requests.push((url.to_owned(), offset));
        if let Some(to) = state.redirects.get(url) {
            return Ok(Opened::Redirect(to.clone()));
        }
        let body = state.files.get(url).cloned().ok_or_else(|| PluginError::Network(format!("404 {url}")))?;
        let total = body.len() as u64;
        let (resumed, start) = if state.no_range || offset == 0 { (false, 0) } else { (true, offset as usize) };
        let mut rest = body[start.min(body.len())..].to_vec();
        let cut = state.cut_after.remove(url);
        if let Some(cut) = cut {
            rest.truncate(cut);
        }
        let (held, gate) = match state.hold_after.remove(url) {
            Some((bytes, gate)) => (rest.split_off(bytes.min(rest.len())), Some(gate)),
            None => (Vec::new(), None),
        };
        // Small chunks, so progress has several steps.
        let chunks_of =
            |bytes: &[u8]| bytes.chunks(4096).map(|c| Ok(c.to_vec())).collect::<Vec<PluginResult<Vec<u8>>>>();
        let mut stream = futures::stream::iter(chunks_of(&rest)).boxed();
        if let Some(mut gate) = gate {
            let wait = futures::stream::once(async move {
                let _ = gate.wait_for(|open| *open).await;
            })
            .filter_map(|()| async { None });
            stream = stream.chain(wait).chain(futures::stream::iter(chunks_of(&held))).boxed();
        }
        if cut.is_some() {
            stream =
                stream.chain(futures::stream::iter([Err(PluginError::Network("connection reset".into()))])).boxed();
        }
        Ok(Opened::Body(Body { resumed, total: Some(total), chunks: stream }))
    }
}

/// A process the fake spawner started.
#[derive(Clone)]
pub struct SpawnedProcess {
    pub command: SpawnCommand,
    /// The process id the fake gave it.
    pub id: u32,
    exit: Arc<Mutex<Option<oneshot::Sender<ProcessExit>>>>,
    signals: Arc<Mutex<Vec<Signal>>>,
}

/// What the host did to a process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    /// Asked it to close.
    Terminate,
    Kill,
}

impl SpawnedProcess {
    /// Makes the process exit with `code`.
    pub fn exit(&self, code: i32) {
        if let Some(tx) = self.exit.lock().unwrap().take() {
            let _ = tx.send(ProcessExit { code: Some(code), success: code == 0 });
        }
    }

    /// What the host did to it, in order.
    pub fn signals(&self) -> Vec<Signal> {
        self.signals.lock().unwrap().clone()
    }
}

/// Records spawn requests instead of starting programs. Each process runs until the test
/// calls [`SpawnedProcess::exit`], or the host kills it.
///
/// Programs run for their output ([`ProcessSpawner::output`]) print what the test set with
/// [`FakeSpawner::prints`], or hang with [`FakeSpawner::hangs`]. `PATH` is what the test set
/// with [`FakeSpawner::on_path`].
#[derive(Clone)]
pub struct FakeSpawner {
    spawned: Arc<Mutex<Vec<SpawnedProcess>>>,
    count: watch::Sender<usize>,
    outputs: Arc<Mutex<std::collections::HashMap<std::path::PathBuf, Option<Vec<u8>>>>>,
    path: Arc<Mutex<std::collections::HashMap<String, std::path::PathBuf>>>,
    ran: Arc<Mutex<Vec<SpawnCommand>>>,
    ignores_terminate: Arc<std::sync::atomic::AtomicBool>,
}

/// The first process id the fake gives out.
pub const FIRST_PID: u32 = 1000;

impl Default for FakeSpawner {
    fn default() -> Self {
        Self {
            spawned: Arc::default(),
            count: watch::channel(0).0,
            outputs: Arc::default(),
            path: Arc::default(),
            ran: Arc::default(),
            ignores_terminate: Arc::default(),
        }
    }
}

impl FakeSpawner {
    /// The program at `program` prints `output` and exits.
    pub fn prints(&self, program: &std::path::Path, output: &[u8]) {
        self.outputs.lock().unwrap().insert(program.to_path_buf(), Some(output.to_vec()));
    }

    /// The program at `program` never ends.
    pub fn hangs(&self, program: &std::path::Path) {
        self.outputs.lock().unwrap().insert(program.to_path_buf(), None);
    }

    /// `name` is on `PATH`, at `path`.
    pub fn on_path(&self, name: &str, path: &std::path::Path) {
        self.path.lock().unwrap().insert(name.into(), path.to_path_buf());
    }

    /// Processes spawned from now on don't close when asked: only a kill stops them.
    pub fn ignore_terminate(&self) {
        self.ignores_terminate.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// The programs run for their output, in order.
    pub fn ran(&self) -> Vec<SpawnCommand> {
        self.ran.lock().unwrap().clone()
    }

    pub fn spawned(&self) -> Vec<SpawnedProcess> {
        self.spawned.lock().unwrap().clone()
    }

    /// Waits until at least `n` processes were spawned.
    pub async fn wait_for(&self, n: usize) -> SpawnedProcess {
        let mut rx = self.count.subscribe();
        rx.wait_for(|count| *count >= n).await.expect("the spawner lives as long as the test");
        self.spawned()[n - 1].clone()
    }
}

struct FakeProcess {
    id: u32,
    exit: Option<oneshot::Receiver<ProcessExit>>,
    tx: Arc<Mutex<Option<oneshot::Sender<ProcessExit>>>>,
    signals: Arc<Mutex<Vec<Signal>>>,
    ignores_terminate: bool,
}

#[async_trait]
impl ProcessSpawner for FakeSpawner {
    async fn spawn(&self, command: SpawnCommand) -> PluginResult<Box<dyn ProcessHandle>> {
        let (tx, rx) = oneshot::channel();
        let tx = Arc::new(Mutex::new(Some(tx)));
        let signals = Arc::<Mutex<Vec<Signal>>>::default();
        let mut spawned = self.spawned.lock().unwrap();
        let id = FIRST_PID + spawned.len() as u32;
        spawned.push(SpawnedProcess { command, id, exit: tx.clone(), signals: signals.clone() });
        drop(spawned);
        self.count.send_modify(|c| *c += 1);
        let ignores_terminate = self.ignores_terminate.load(std::sync::atomic::Ordering::SeqCst);
        Ok(Box::new(FakeProcess { id, exit: Some(rx), tx, signals, ignores_terminate }))
    }

    async fn output(
        &self,
        command: SpawnCommand,
        _timeout: std::time::Duration,
        limit: usize,
    ) -> PluginResult<Option<Vec<u8>>> {
        let output = self.outputs.lock().unwrap().get(&command.program).cloned();
        self.ran.lock().unwrap().push(command.clone());
        match output {
            Some(Some(mut out)) => {
                out.truncate(limit);
                Ok(Some(out))
            }
            Some(None) => Ok(None),
            None => Err(PluginError::Other(format!("the test gave {:?} no output", command.program))),
        }
    }

    fn which(&self, name: &str) -> Option<std::path::PathBuf> {
        self.path.lock().unwrap().get(name).cloned()
    }
}

#[async_trait]
impl ProcessHandle for FakeProcess {
    fn id(&self) -> Option<u32> {
        self.exit.is_some().then_some(self.id)
    }

    async fn terminate(&mut self) -> PluginResult<()> {
        self.signals.lock().unwrap().push(Signal::Terminate);
        if !self.ignores_terminate
            && let Some(tx) = self.tx.lock().unwrap().take()
        {
            let _ = tx.send(ProcessExit { code: None, success: false });
        }
        Ok(())
    }

    async fn wait(&mut self) -> PluginResult<ProcessExit> {
        let rx = self.exit.as_mut().ok_or_else(|| PluginError::Other("already waited".into()))?;
        rx.await.map_err(|_| PluginError::Other("the test dropped the process".into()))
    }

    async fn kill(&mut self) -> PluginResult<()> {
        self.signals.lock().unwrap().push(Signal::Kill);
        self.exit = None;
        Ok(())
    }
}

/// A game's processes as the test says: the one the launcher started, and the ones the test
/// adds below it, as a start script that starts the game in the background, or a sandbox.
#[derive(Clone, Default)]
pub struct FakeProcessTree {
    state: Arc<Mutex<TreeState>>,
}

#[derive(Default)]
struct TreeState {
    /// The process the launcher started -> the processes below it.
    below: std::collections::BTreeMap<u32, Vec<u32>>,
    /// They don't close when asked, only when killed.
    stubborn: bool,
    /// Each `end`: the processes, and whether it killed them.
    ended: Vec<(Vec<u32>, bool)>,
    /// Processes the test started on their own, as a store's client starts a game: id ->
    /// command line.
    started: std::collections::BTreeMap<u32, String>,
    /// Those of them that ended.
    gone: std::collections::BTreeSet<u32>,
}

impl FakeProcessTree {
    /// Process `id` runs below the game started as `root`.
    pub fn add(&self, root: u32, id: u32) {
        self.state.lock().unwrap().below.entry(root).or_default().push(id);
    }

    /// Process `id` starts with `command_line`, not below anything Nexus started: a game a
    /// store's client started.
    pub fn start(&self, id: u32, command_line: &str) {
        self.state.lock().unwrap().started.insert(id, command_line.into());
    }

    /// Process `id` ended by itself.
    pub fn exit(&self, id: u32) {
        let mut state = self.state.lock().unwrap();
        if state.started.remove(&id).is_some() {
            state.gone.insert(id);
        }
        for ids in state.below.values_mut() {
            ids.retain(|i| *i != id);
        }
    }

    /// The processes below a game don't close when asked to, only when killed.
    pub fn stubborn(&self) {
        self.state.lock().unwrap().stubborn = true;
    }

    /// Each `end` the core asked for: the processes, and whether it killed them.
    pub fn ended(&self) -> Vec<(Vec<u32>, bool)> {
        self.state.lock().unwrap().ended.clone()
    }
}

impl nexus_core::ProcessTree for FakeProcessTree {
    fn members(&self, root: u32) -> Vec<u32> {
        let state = self.state.lock().unwrap();
        let below = state.below.get(&root).cloned().unwrap_or_default();
        // A process the test started is gone once it ends; whether the launcher's own runs is
        // the fake handle's to say, and the core drops it once it exited.
        let root = (!state.gone.contains(&root)).then_some(root);
        root.into_iter().chain(below).collect()
    }

    fn find(&self, text: &str) -> Vec<u32> {
        self.state.lock().unwrap().started.iter().filter(|(_, line)| line.contains(text)).map(|(id, _)| *id).collect()
    }

    fn end(&self, ids: &[u32], force: bool) {
        let mut state = self.state.lock().unwrap();
        state.ended.push((ids.to_vec(), force));
        if force || !state.stubborn {
            for below in state.below.values_mut() {
                below.retain(|i| !ids.contains(i));
            }
            for id in ids {
                if state.started.remove(id).is_some() {
                    state.gone.insert(*id);
                }
            }
        }
    }
}

/// What the host brought to the front.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shown {
    Game(u32),
    Launcher,
}

/// A desktop the test controls. A game's window shows at once unless the test calls
/// [`FakeDesktop::no_windows`]. Bringing a window to the front puts it in front, until the
/// test says otherwise with [`FakeDesktop::set_front`].
#[derive(Clone)]
pub struct FakeDesktop {
    front: Arc<Mutex<Front>>,
    windows: Arc<std::sync::atomic::AtomicBool>,
    shown: Arc<Mutex<Vec<Shown>>>,
    problem: Arc<Mutex<Option<String>>>,
    /// The build can start at login, as an installed one.
    login: Arc<std::sync::atomic::AtomicBool>,
    logins: Arc<Mutex<Vec<bool>>>,
    login_refusal: Arc<Mutex<Option<String>>>,
    packages: Arc<Mutex<Vec<(std::path::PathBuf, nexus_core::PackageManager)>>>,
    package_refusal: Arc<Mutex<Option<nexus_core::PackageError>>>,
    /// A game whose windows all closed is over, as on macOS.
    window_ends_game: Arc<std::sync::atomic::AtomicBool>,
    /// Each link the core opened.
    links: Arc<Mutex<Vec<String>>>,
}

impl Default for FakeDesktop {
    fn default() -> Self {
        Self {
            front: Arc::new(Mutex::new(Front::Launcher)),
            windows: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            shown: Arc::default(),
            problem: Arc::default(),
            login: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            logins: Arc::default(),
            login_refusal: Arc::default(),
            packages: Arc::default(),
            package_refusal: Arc::default(),
            window_ends_game: Arc::default(),
            links: Arc::default(),
        }
    }
}

impl FakeDesktop {
    /// The desktop's problem, for example a missing tool: what `Desktop::problem` says.
    pub fn set_problem(&self, problem: &str) {
        *self.problem.lock().unwrap() = Some(problem.into());
    }

    pub fn set_front(&self, front: Front) {
        *self.front.lock().unwrap() = front;
    }

    /// Games have no window to show: none yet, or the player closed it.
    pub fn no_windows(&self) {
        self.windows.store(false, std::sync::atomic::Ordering::SeqCst);
    }

    /// A game whose windows all closed is over, as on macOS, where a program outlives them.
    pub fn windows_end_games(&self) {
        self.window_ends_game.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Games' windows show again, as when a slow game's window finally opens.
    pub fn windows_open(&self) {
        self.windows.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// The build can't start at login, as one that isn't installed.
    pub fn no_login(&self) {
        self.login.store(false, std::sync::atomic::Ordering::SeqCst);
    }

    /// The system refuses each change to start at login, with `error`.
    pub fn refuse_login(&self, error: &str) {
        *self.login_refusal.lock().unwrap() = Some(error.into());
    }

    /// Each change to start at login the system carried out, in order.
    pub fn logins(&self) -> Vec<bool> {
        self.logins.lock().unwrap().clone()
    }

    /// What the host brought to the front, in order.
    pub fn shown(&self) -> Vec<Shown> {
        self.shown.lock().unwrap().clone()
    }

    /// The package manager refuses each launcher package with `error`.
    pub fn refuse_packages(&self, error: nexus_core::PackageError) {
        *self.package_refusal.lock().unwrap() = Some(error);
    }

    /// Each launcher package the package manager installed, with the file it was given (which
    /// existed then), in order.
    pub fn packages(&self) -> Vec<(std::path::PathBuf, nexus_core::PackageManager)> {
        self.packages.lock().unwrap().clone()
    }

    /// Each link the core opened, in order.
    pub fn links(&self) -> Vec<String> {
        self.links.lock().unwrap().clone()
    }
}

impl Desktop for FakeDesktop {
    fn front(&self) -> Front {
        *self.front.lock().unwrap()
    }

    fn show_game(&self, ids: &[u32]) -> bool {
        let Some(&id) = ids.first() else { return false };
        if !self.windows.load(std::sync::atomic::Ordering::SeqCst) {
            return false;
        }
        self.shown.lock().unwrap().push(Shown::Game(id));
        let mut front = self.front.lock().unwrap();
        // A desktop that doesn't say which window is in front still doesn't say.
        if *front != Front::Unknown {
            *front = Front::Other(id);
        }
        true
    }

    fn problem(&self) -> Option<String> {
        self.problem.lock().unwrap().clone()
    }

    fn can_start_at_login(&self) -> bool {
        self.login.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn start_at_login(&self, on: bool) -> Result<(), String> {
        if let Some(error) = self.login_refusal.lock().unwrap().clone() {
            return Err(error);
        }
        self.logins.lock().unwrap().push(on);
        Ok(())
    }

    fn has_window(&self, _ids: &[u32]) -> Option<bool> {
        Some(self.windows.load(std::sync::atomic::Ordering::SeqCst))
    }

    fn window_close_ends_game(&self) -> bool {
        self.window_ends_game.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn install_package(
        &self,
        file: &std::path::Path,
        manager: nexus_core::PackageManager,
    ) -> Result<(), nexus_core::PackageError> {
        if let Some(error) = self.package_refusal.lock().unwrap().clone() {
            return Err(error);
        }
        assert!(file.is_file(), "the package manager is given a file that exists: {}", file.display());
        self.packages.lock().unwrap().push((file.to_path_buf(), manager));
        Ok(())
    }

    fn show_launcher(&self) {
        self.shown.lock().unwrap().push(Shown::Launcher);
        let mut front = self.front.lock().unwrap();
        if *front != Front::Unknown {
            *front = Front::Launcher;
        }
    }

    fn open_link(&self, link: &str) -> Result<(), String> {
        self.links.lock().unwrap().push(link.to_owned());
        Ok(())
    }
}
