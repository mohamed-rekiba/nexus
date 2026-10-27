//! The download manager: a long-lived task that fetches the files of a [`DownloadPlan`] into a
//! plugin's data folder.
//!
//! The app talks to it through a channel: submit, pause, resume, cancel. The task owns every
//! job. It runs at most [`HostConfig::max_downloads`](crate::HostConfig) at once and queues the
//! rest. Each running job is a worker task that reports progress back on a second channel.
//! The task publishes a snapshot of the jobs after each change, which the view reads.
//!
//! Each file goes to `<dest>.part` first. A later attempt continues from the end of that file
//! with a Range request; if the server sends the whole file instead, the part starts over.
//! When all bytes are in, the SHA-256 (and the signature, if any) must match before the part
//! becomes `<dest>`. A file that fails the check is deleted. A paused job keeps its part
//! files; a cancelled job deletes them.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::StreamExt;
use nexus_api::{
    Artifact, DownloadPlan, DownloadedArtifact, Event, JobId, Permissions, PluginError, PluginId, PluginResult,
    SignatureAlgorithm,
};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot, watch};

use crate::facades::check_url;
use crate::{
    Clock, ConfigService, CoreError, CoreResult, DirStorage, EventBus, HostConfig, HttpTransport, ProcessSpawner,
};

/// Where a download job is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobPhase {
    Queued,
    Downloading,
    Verifying,
    Unpacking,
    Paused,
    Done,
    Failed,
    Cancelled,
}

impl JobPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Downloading => "downloading",
            Self::Verifying => "verifying",
            Self::Unpacking => "unpacking",
            Self::Paused => "paused",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    /// The job will not run again.
    pub fn is_over(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }
}

#[derive(Clone, Debug)]
pub struct Job {
    pub id: JobId,
    pub plugin: PluginId,
    pub label: String,
    pub received: u64,
    pub total: Option<u64>,
    pub phase: JobPhase,
    pub error: Option<String>,
}

/// The result of a job: the checked files.
pub(crate) type Outcome = CoreResult<Vec<DownloadedArtifact>>;

enum Msg {
    Submit {
        id: JobId,
        plugin: PluginId,
        plan: DownloadPlan,
        saved_as: Option<String>,
        reply: oneshot::Sender<Outcome>,
    },
    Pause(JobId, oneshot::Sender<CoreResult<()>>),
    Resume(JobId, oneshot::Sender<CoreResult<()>>),
    Cancel(JobId, oneshot::Sender<CoreResult<()>>),
}

/// What a worker tells the manager.
enum Report {
    Phase(JobId, JobPhase),
    Total(JobId, u64),
    Progress(JobId, u64),
    Ended(JobId, Result<Vec<DownloadedArtifact>, Stopped>),
}

/// Why a worker ended without the files.
enum Stopped {
    /// The manager asked it to stop.
    Asked,
    Error(CoreError),
}

/// The ports the download manager takes: the files come over HTTP, and an AppImage runs to
/// unpack itself.
pub(crate) struct DownloadPorts {
    pub http: Arc<dyn HttpTransport>,
    pub spawner: Arc<dyn ProcessSpawner>,
    pub clock: Arc<dyn Clock>,
}

/// What the download manager reads besides its ports: where each plugin's data folder is,
/// what each plugin may fetch, and where it saves and announces its jobs.
pub(crate) struct DownloadContext {
    pub ports: DownloadPorts,
    /// The data folder, the system, and how many downloads run at once.
    pub host: HostConfig,
    /// Each loaded plugin's permissions, as its manifest declares them.
    pub permissions: BTreeMap<PluginId, Permissions>,
    /// Holds a job's record under `app.downloads` until it ends.
    pub config: Arc<ConfigService>,
    pub bus: EventBus,
}

impl DownloadContext {
    fn permissions(&self, plugin: &PluginId) -> CoreResult<&Permissions> {
        self.permissions.get(plugin).ok_or_else(|| CoreError::UnknownPlugin(plugin.clone()))
    }
}

/// The app's handle on the download manager. Dropping it stops the manager; running jobs stop
/// where they are and keep their part files.
pub(crate) struct Downloads {
    next: AtomicU64,
    tx: mpsc::UnboundedSender<Msg>,
    jobs: watch::Receiver<BTreeMap<JobId, Job>>,
}

impl Downloads {
    /// Starts the manager task. Needs a Tokio runtime.
    pub fn spawn(context: DownloadContext) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let (jobs_tx, jobs) = watch::channel(BTreeMap::new());
        let (reports_tx, reports) = mpsc::unbounded_channel();
        let max_running = context.host.max_downloads.max(1);
        let manager = Manager {
            context: Arc::new(context),
            max_running,
            jobs: jobs_tx,
            entries: BTreeMap::new(),
            queue: VecDeque::new(),
            reports_tx,
        };
        tokio::spawn(manager.run(rx, reports));
        Self { next: AtomicU64::new(0), tx, jobs }
    }

    /// A new job id. Game installs take theirs from here too, so Downloads lists one id space.
    pub(crate) fn next_id(&self) -> JobId {
        JobId(self.next.fetch_add(1, Ordering::SeqCst) + 1)
    }

    pub fn jobs(&self) -> Vec<Job> {
        self.jobs.borrow().values().cloned().collect()
    }

    /// Queues the plan as one job. The receiver gets the checked files when the job ends.
    /// `saved_as` is the key under `app.downloads` that holds the job's record; the manager
    /// removes it when the job ends.
    pub fn submit(
        &self,
        plugin: &PluginId,
        plan: DownloadPlan,
        saved_as: Option<String>,
    ) -> (JobId, oneshot::Receiver<Outcome>) {
        let id = self.next_id();
        let (reply, rx) = oneshot::channel();
        // A closed channel means the app is stopping; the receiver then reports that.
        let _ = self.tx.send(Msg::Submit { id, plugin: plugin.clone(), plan, saved_as, reply });
        (id, rx)
    }

    /// Runs the plan and waits for the checked files.
    pub async fn run(&self, plugin: &PluginId, plan: DownloadPlan) -> Outcome {
        let (_, rx) = self.submit(plugin, plan, None);
        rx.await.unwrap_or_else(|_| Err(stopping()))
    }

    pub async fn pause(&self, job: JobId) -> CoreResult<()> {
        self.ask(|reply| Msg::Pause(job, reply)).await
    }

    pub async fn resume(&self, job: JobId) -> CoreResult<()> {
        self.ask(|reply| Msg::Resume(job, reply)).await
    }

    pub async fn cancel(&self, job: JobId) -> CoreResult<()> {
        self.ask(|reply| Msg::Cancel(job, reply)).await
    }

    async fn ask(&self, msg: impl FnOnce(oneshot::Sender<CoreResult<()>>) -> Msg) -> CoreResult<()> {
        let (reply, rx) = oneshot::channel();
        self.tx.send(msg(reply)).map_err(|_| stopping())?;
        rx.await.unwrap_or_else(|_| Err(stopping()))
    }
}

fn stopping() -> CoreError {
    CoreError::Download("the download manager stopped".into())
}

struct Entry {
    plugin: PluginId,
    plan: DownloadPlan,
    saved_as: Option<String>,
    reply: Option<oneshot::Sender<Outcome>>,
    /// Set while a worker runs. Sending `true` asks it to stop.
    stop: Option<watch::Sender<bool>>,
    /// What to do when the worker has stopped.
    after_stop: AfterStop,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AfterStop {
    Nothing,
    Pause,
    Cancel,
}

struct Manager {
    context: Arc<DownloadContext>,
    max_running: usize,
    jobs: watch::Sender<BTreeMap<JobId, Job>>,
    entries: BTreeMap<JobId, Entry>,
    queue: VecDeque<JobId>,
    reports_tx: mpsc::UnboundedSender<Report>,
}

impl Manager {
    async fn run(mut self, mut rx: mpsc::UnboundedReceiver<Msg>, mut reports: mpsc::UnboundedReceiver<Report>) {
        loop {
            tokio::select! {
                msg = rx.recv() => match msg {
                    Some(msg) => self.handle(msg),
                    None => break,
                },
                Some(report) = reports.recv() => self.report(report).await,
            }
            self.start_queued();
        }
        // The app is stopping: stop every worker where it is. Saved records stay, so the next
        // start resumes these jobs.
        for entry in self.entries.values() {
            if let Some(stop) = &entry.stop {
                let _ = stop.send(true);
            }
        }
    }

    fn set(&self, id: JobId, f: impl FnOnce(&mut Job)) {
        self.jobs.send_modify(|jobs| {
            if let Some(job) = jobs.get_mut(&id) {
                f(job);
            }
        });
    }

    fn phase(&self, id: JobId) -> Option<JobPhase> {
        self.jobs.borrow().get(&id).map(|j| j.phase)
    }

    fn handle(&mut self, msg: Msg) {
        let bus = self.context.bus.clone();
        match msg {
            Msg::Submit { id, plugin, plan, saved_as, reply } => {
                let total = plan.artifacts.iter().map(|a| a.size).sum::<Option<u64>>();
                let job = Job {
                    id,
                    plugin: plugin.clone(),
                    label: plan.label.clone(),
                    received: 0,
                    total,
                    phase: JobPhase::Queued,
                    error: None,
                };
                self.jobs.send_modify(|jobs| {
                    jobs.insert(id, job);
                });
                bus.publish(Event::DownloadQueued { job: id, plugin: plugin.clone(), label: plan.label.clone() });
                self.entries.insert(
                    id,
                    Entry { plugin, plan, saved_as, reply: Some(reply), stop: None, after_stop: AfterStop::Nothing },
                );
                self.queue.push_back(id);
            }
            Msg::Pause(id, reply) => {
                let _ = reply.send(self.pause(id));
            }
            Msg::Resume(id, reply) => {
                let _ = reply.send(self.resume(id));
            }
            Msg::Cancel(id, reply) => {
                let _ = reply.send(self.cancel(id));
            }
        }
    }

    fn pause(&mut self, id: JobId) -> CoreResult<()> {
        let phase = self.phase(id).ok_or(CoreError::UnknownJob(id))?;
        if !matches!(phase, JobPhase::Queued | JobPhase::Downloading) {
            return Err(CoreError::Invalid(format!("download {id} is {}, so it can't pause", phase.as_str())));
        }
        let entry = self.entries.get_mut(&id).ok_or(CoreError::UnknownJob(id))?;
        match &entry.stop {
            // Paused once the worker has stopped and written what it has.
            Some(stop) => {
                let _ = stop.send(true);
                entry.after_stop = AfterStop::Pause;
            }
            None => {
                self.queue.retain(|queued| *queued != id);
                self.paused(id);
            }
        }
        Ok(())
    }

    fn resume(&mut self, id: JobId) -> CoreResult<()> {
        let phase = self.phase(id).ok_or(CoreError::UnknownJob(id))?;
        if phase != JobPhase::Paused {
            return Err(CoreError::Invalid(format!("download {id} is {}, not paused", phase.as_str())));
        }
        // A paused job has no worker, so it goes straight back in the queue.
        self.queue.push_back(id);
        self.set(id, |j| j.phase = JobPhase::Queued);
        self.context.bus.publish(Event::DownloadResumed { job: id });
        Ok(())
    }

    fn cancel(&mut self, id: JobId) -> CoreResult<()> {
        let phase = self.phase(id).ok_or(CoreError::UnknownJob(id))?;
        if phase.is_over() {
            return Err(CoreError::Invalid(format!("download {id} is already {}", phase.as_str())));
        }
        let entry = self.entries.get_mut(&id).ok_or(CoreError::UnknownJob(id))?;
        match &entry.stop {
            Some(stop) => {
                let _ = stop.send(true);
                entry.after_stop = AfterStop::Cancel;
            }
            None => {
                self.queue.retain(|queued| *queued != id);
                self.finish_cancel(id);
            }
        }
        Ok(())
    }

    fn running(&self) -> usize {
        self.entries.values().filter(|e| e.stop.is_some()).count()
    }

    fn start_queued(&mut self) {
        while self.running() < self.max_running {
            let Some(id) = self.queue.pop_front() else { break };
            let Some(entry) = self.entries.get_mut(&id) else { continue };
            let (stop, stop_rx) = watch::channel(false);
            entry.stop = Some(stop);
            entry.after_stop = AfterStop::Nothing;
            let worker = Worker {
                context: self.context.clone(),
                id,
                plugin: entry.plugin.clone(),
                artifacts: entry.plan.artifacts.clone(),
                reports: self.reports_tx.clone(),
                stop: stop_rx,
            };
            tokio::spawn(worker.run());
        }
    }

    async fn report(&mut self, report: Report) {
        match report {
            Report::Phase(id, phase) => {
                // A pause or cancel already set the phase the player sees.
                if self.entries.get(&id).is_some_and(|e| e.after_stop == AfterStop::Nothing) {
                    self.set(id, |j| j.phase = phase);
                }
            }
            Report::Total(id, total) => self.set(id, |j| j.total = Some(total)),
            Report::Progress(id, received) => {
                self.set(id, |j| j.received = received);
                let total = self.jobs.borrow().get(&id).and_then(|j| j.total);
                self.context.bus.publish(Event::DownloadProgress { job: id, received, total });
            }
            Report::Ended(id, result) => self.ended(id, result).await,
        }
    }

    async fn ended(&mut self, id: JobId, result: Result<Vec<DownloadedArtifact>, Stopped>) {
        let Some(entry) = self.entries.get_mut(&id) else { return };
        entry.stop = None;
        let after = std::mem::replace(&mut entry.after_stop, AfterStop::Nothing);
        let bus = self.context.bus.clone();
        match result {
            Ok(files) => {
                self.set(id, |j| j.phase = JobPhase::Done);
                bus.publish(Event::DownloadFinished { job: id });
                self.close(id, Ok(files));
            }
            Err(Stopped::Error(error)) => {
                let message = error.to_string();
                let plugin = &self.entries[&id].plugin;
                tracing::warn!(plugin_id = %plugin, job = %id, error = %message, "download failed");
                self.set(id, |j| {
                    j.phase = JobPhase::Failed;
                    j.error = Some(message.clone());
                });
                bus.publish(Event::DownloadFailed { job: id, error: message });
                self.close(id, Err(error));
            }
            Err(Stopped::Asked) => match after {
                AfterStop::Cancel => {
                    self.delete_parts(id).await;
                    self.finish_cancel(id);
                }
                // The part files stay for a resume.
                AfterStop::Pause => self.paused(id),
                // Only the manager stopping asks without a reason, and then nothing runs on.
                AfterStop::Nothing => {}
            },
        }
    }

    fn paused(&self, id: JobId) {
        self.set(id, |j| j.phase = JobPhase::Paused);
        self.context.bus.publish(Event::DownloadPaused { job: id });
    }

    fn finish_cancel(&mut self, id: JobId) {
        self.set(id, |j| j.phase = JobPhase::Cancelled);
        self.context.bus.publish(Event::DownloadCancelled { job: id });
        self.close(id, Err(CoreError::Cancelled(id)));
    }

    /// Ends a job for good: forgets its record and tells whoever waits for it.
    fn close(&mut self, id: JobId, outcome: Outcome) {
        let Some(mut entry) = self.entries.remove(&id) else { return };
        if let Some(key) = entry.saved_as.take()
            && let Err(e) = self.context.config.remove(&["app", "downloads", &key])
        {
            tracing::warn!(error = %e, "could not save download state");
        }
        if let Some(reply) = entry.reply.take() {
            let _ = reply.send(outcome);
        }
    }

    async fn delete_parts(&self, id: JobId) {
        let Some(entry) = self.entries.get(&id) else { return };
        let storage = DirStorage::new(self.context.host.plugin_dir(&entry.plugin));
        for artifact in &entry.plan.artifacts {
            if let Ok(dest) = storage.resolve(&artifact.dest) {
                let _ = tokio::fs::remove_file(part_path(&dest)).await;
            }
        }
    }
}

/// Runs one job's files in order and reports back.
struct Worker {
    context: Arc<DownloadContext>,
    id: JobId,
    plugin: PluginId,
    artifacts: Vec<Artifact>,
    reports: mpsc::UnboundedSender<Report>,
    stop: watch::Receiver<bool>,
}

impl Worker {
    async fn run(mut self) {
        let result = self.run_job().await;
        let _ = self.reports.send(Report::Ended(self.id, result));
    }

    fn report(&self, report: Report) {
        let _ = self.reports.send(report);
    }

    async fn run_job(&mut self) -> Result<Vec<DownloadedArtifact>, Stopped> {
        let failed = |e: String| Stopped::Error(CoreError::Download(e));
        let permissions = self.context.permissions(&self.plugin).map_err(Stopped::Error)?;
        // Check every address before fetching anything.
        for artifact in &self.artifacts {
            check_url(&self.plugin, permissions, &artifact.url).map_err(|e| failed(e.to_string()))?;
        }
        let storage = DirStorage::new(self.context.host.plugin_dir(&self.plugin));
        let mut done_before = 0u64;
        let mut out = Vec::new();
        for artifact in self.artifacts.clone() {
            let dest = storage.resolve(&artifact.dest).map_err(|e| failed(e.to_string()))?;
            let sha256 = artifact.sha256.to_lowercase();
            let Some(unpack) = &artifact.unpack else {
                let size = self.fetch(&artifact, &dest, done_before).await?;
                done_before += size;
                out.push(DownloadedArtifact { dest: artifact.dest.clone(), sha256, size, unpacked: None });
                continue;
            };
            let into = storage.resolve(&unpack.into).map_err(|e| failed(e.to_string()))?;
            // Already unpacked from this very file (the app stopped after the unpack).
            let size = match unpacked_from(&into, &sha256).await {
                Some(size) => size,
                None => {
                    let size = self.fetch(&artifact, &dest, done_before).await?;
                    self.report(Report::Phase(self.id, JobPhase::Unpacking));
                    let root = storage.root().to_path_buf();
                    let permissions = self.context.permissions(&self.plugin).map_err(Stopped::Error)?.clone();
                    let may_run = move |path: &Path| {
                        let rel = path.strip_prefix(&root).ok().map(|p| p.to_string_lossy().replace('\\', "/"));
                        rel.and_then(|r| nexus_api::RelPath::new(r).ok())
                            .is_some_and(|r| permissions.allows_program(&nexus_api::Location::Data(r)))
                    };
                    crate::unpack::unpack(
                        &self.context.ports,
                        self.context.host.platform,
                        &may_run,
                        unpack.format,
                        &dest,
                        &into,
                    )
                    .await
                    .map_err(Stopped::Error)?;
                    tokio::fs::write(into.join(UNPACKED_MARK), format!("{sha256} {size}")).await.map_err(io)?;
                    let _ = tokio::fs::remove_file(&dest).await;
                    size
                }
            };
            done_before += size;
            out.push(DownloadedArtifact {
                dest: artifact.dest.clone(),
                sha256,
                size,
                unpacked: Some(unpack.into.clone()),
            });
        }
        Ok(out)
    }

    /// Fetches one file to `dest` and returns its size. A `dest` that is already there and
    /// passes the checks is kept.
    async fn fetch(&mut self, artifact: &Artifact, dest: &Path, done_before: u64) -> Result<u64, Stopped> {
        if let Ok(meta) = tokio::fs::metadata(dest).await
            && meta.is_file()
        {
            self.report(Report::Phase(self.id, JobPhase::Verifying));
            if verify(dest, artifact).await.is_ok() {
                return Ok(meta.len());
            }
            let _ = tokio::fs::remove_file(dest).await;
        }
        let part = part_path(dest);
        if let Some(parent) = part.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(io)?;
        }
        let offset = tokio::fs::metadata(&part).await.map(|m| m.len()).unwrap_or(0);
        self.report(Report::Phase(self.id, JobPhase::Downloading));
        // A part that already has every byte (the app stopped before the check) needs no
        // request: servers refuse a range that starts at the end of the file.
        let complete = artifact.size.is_some_and(|size| offset >= size);
        let received = if complete { offset } else { self.receive(artifact, &part, offset, done_before).await? };

        self.report(Report::Phase(self.id, JobPhase::Verifying));
        if let Err(e) = verify(&part, artifact).await {
            let _ = tokio::fs::remove_file(&part).await;
            return Err(Stopped::Error(e));
        }
        tokio::fs::rename(&part, dest).await.map_err(io)?;
        Ok(received)
    }

    /// Opens `url` at `offset`, following redirects that stay in the allowlist.
    async fn open(&self, url: &str, offset: u64) -> PluginResult<crate::Body> {
        let permissions = self.context.permissions(&self.plugin).map_err(|e| PluginError::Other(e.to_string()))?;
        let mut url = url.to_owned();
        for _ in 0..=crate::facades::MAX_REDIRECTS {
            match self.context.ports.http.open(&url, offset).await? {
                crate::Opened::Body(body) => return Ok(body),
                crate::Opened::Redirect(location) => {
                    url = crate::facades::next_hop(&self.plugin, permissions, &url, &location)?;
                }
            }
        }
        Err(PluginError::Network(format!("more than {} redirects", crate::facades::MAX_REDIRECTS)))
    }

    /// Downloads the rest of `artifact` into `part`, from `offset`. Returns the part's size.
    /// Stops between chunks when the manager asks, with every byte so far written.
    async fn receive(
        &mut self,
        artifact: &Artifact,
        part: &Path,
        mut offset: u64,
        done_before: u64,
    ) -> Result<u64, Stopped> {
        let body =
            self.open(&artifact.url, offset).await.map_err(|e| Stopped::Error(CoreError::Download(e.to_string())))?;
        let mut file = if body.resumed && offset > 0 {
            tokio::fs::OpenOptions::new().append(true).open(part).await.map_err(io)?
        } else {
            offset = 0;
            tokio::fs::File::create(part).await.map_err(io)?
        };
        if let Some(total) = body.total
            && artifact.size.is_none()
        {
            self.report(Report::Total(self.id, done_before + total));
        }

        let mut received = offset;
        let mut chunks = body.chunks;
        loop {
            let stop = &mut self.stop;
            let chunk = tokio::select! {
                biased;
                () = async { let _ = stop.wait_for(|stop| *stop).await; } => {
                    file.flush().await.map_err(io)?;
                    return Err(Stopped::Asked);
                }
                chunk = chunks.next() => chunk,
            };
            let Some(chunk) = chunk else { break };
            let chunk = chunk.map_err(|e| Stopped::Error(CoreError::Download(e.to_string())))?;
            file.write_all(&chunk).await.map_err(io)?;
            received += chunk.len() as u64;
            self.report(Report::Progress(self.id, done_before + received));
        }
        file.flush().await.map_err(io)?;
        Ok(received)
    }
}

/// Written into an unpacked folder: the archive's SHA-256 and size.
const UNPACKED_MARK: &str = ".unpacked";

/// The archive's size, when `into` holds an unpack of the archive with this SHA-256.
async fn unpacked_from(into: &Path, sha256: &str) -> Option<u64> {
    let mark = tokio::fs::read_to_string(into.join(UNPACKED_MARK)).await.ok()?;
    let (sha, size) = mark.trim().split_once(' ')?;
    (sha == sha256).then(|| size.parse().ok()).flatten()
}

fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".part");
    dest.with_file_name(name)
}

fn io(e: std::io::Error) -> Stopped {
    Stopped::Error(CoreError::Io(e.to_string()))
}

fn io_error(e: std::io::Error) -> CoreError {
    CoreError::Io(e.to_string())
}

async fn verify(path: &Path, artifact: &Artifact) -> CoreResult<()> {
    let mut file = tokio::fs::File::open(path).await.map_err(io_error)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buffer).await.map_err(io_error)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    let actual = hex::encode(hasher.finalize());
    if !actual.eq_ignore_ascii_case(&artifact.sha256) {
        return Err(CoreError::Download(format!(
            "checksum mismatch for {}: expected {}, got {actual}",
            artifact.dest, artifact.sha256
        )));
    }
    if let Some(signature) = &artifact.signature {
        match signature.algorithm {
            SignatureAlgorithm::Ed25519 => {
                let bytes = tokio::fs::read(path).await.map_err(io_error)?;
                verify_ed25519(&bytes, &signature.public_key, &signature.signature)
                    .map_err(|e| CoreError::Download(format!("bad signature for {}: {e}", artifact.dest)))?;
            }
        }
    }
    Ok(())
}

fn verify_ed25519(message: &[u8], public_key_hex: &str, signature_hex: &str) -> Result<(), String> {
    let key: [u8; 32] = hex::decode(public_key_hex)
        .map_err(|e| e.to_string())?
        .try_into()
        .map_err(|_| "the public key must be 32 bytes".to_owned())?;
    let key = ed25519_dalek::VerifyingKey::from_bytes(&key).map_err(|e| e.to_string())?;
    let signature = hex::decode(signature_hex).map_err(|e| e.to_string())?;
    let signature = ed25519_dalek::Signature::from_slice(&signature).map_err(|e| e.to_string())?;
    key.verify_strict(message, &signature).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signature_from_another_key_or_message_fails() {
        // RFC 8032, section 7.1, test 1: empty message.
        let key = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
        let sig = "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b";
        assert!(verify_ed25519(b"", key, sig).is_ok());
        assert!(verify_ed25519(b"x", key, sig).is_err());
        assert!(verify_ed25519(b"", &"00".repeat(32), sig).is_err());
    }
}
