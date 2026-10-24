use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use nexus_api::{
    EmulatorPlugin, Plugin, PluginByKind, PluginContext, PluginError, PluginManifest, PluginResult, Provided,
};

type InitFn = Arc<dyn Fn(PluginContext) -> PluginResult<Provided> + Send + Sync>;

#[derive(Clone)]
enum Behaviour {
    Provide(InitFn),
    Panic,
    Hang,
    Fail(PluginError),
}

/// A plugin made from a manifest plus test-chosen behaviour. It counts its `init` and
/// `shutdown` calls.
#[derive(Clone)]
pub struct StubPlugin {
    manifest: PluginManifest,
    behaviour: Behaviour,
    inits: Arc<AtomicUsize>,
    shutdowns: Arc<AtomicUsize>,
    shutdown_hangs: bool,
}

impl StubPlugin {
    /// Panics on a bad manifest: it's a test.
    pub fn from_toml(manifest: &str) -> Self {
        let manifest = PluginManifest::from_toml(manifest).unwrap_or_else(|e| panic!("{e}\n{manifest}"));
        Self {
            manifest,
            behaviour: Behaviour::Provide(Arc::new(|_| Ok(Provided::default()))),
            inits: Arc::default(),
            shutdowns: Arc::default(),
            shutdown_hangs: false,
        }
    }

    /// `init` returns what `f` builds from the context.
    pub fn provides(mut self, f: impl Fn(PluginContext) -> PluginResult<Provided> + Send + Sync + 'static) -> Self {
        self.behaviour = Behaviour::Provide(Arc::new(f));
        self
    }

    /// The console the stub emulator plays, as the manifest's `console` says.
    pub fn console(mut self, console: &str) -> Self {
        self.manifest.console = Some(nexus_api::Console::new(console).expect("a console name"));
        self
    }

    pub fn panics_on_init(mut self) -> Self {
        self.behaviour = Behaviour::Panic;
        self
    }

    pub fn hangs_on_init(mut self) -> Self {
        self.behaviour = Behaviour::Hang;
        self
    }

    pub fn fails_on_init(mut self, error: PluginError) -> Self {
        self.behaviour = Behaviour::Fail(error);
        self
    }

    pub fn hangs_on_shutdown(mut self) -> Self {
        self.shutdown_hangs = true;
        self
    }

    pub fn init_calls(&self) -> usize {
        self.inits.load(Ordering::SeqCst)
    }

    pub fn shutdown_calls(&self) -> usize {
        self.shutdowns.load(Ordering::SeqCst)
    }

    /// As the kind its manifest says: an emulator, a service or a system plugin.
    pub fn boxed(&self) -> PluginByKind {
        match self.manifest.kind {
            nexus_api::PluginKind::Service => PluginByKind::service(self.clone()),
            nexus_api::PluginKind::System => PluginByKind::system(self.clone()),
            _ => PluginByKind::emulator(self.clone()),
        }
    }
}

#[async_trait]
impl Plugin for StubPlugin {
    fn manifest(&self) -> PluginManifest {
        self.manifest.clone()
    }

    async fn shutdown(&self) -> PluginResult<()> {
        self.shutdowns.fetch_add(1, Ordering::SeqCst);
        if self.shutdown_hangs {
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
        Ok(())
    }
}

#[async_trait]
impl EmulatorPlugin for StubPlugin {
    async fn init(&self, ctx: PluginContext) -> PluginResult<Provided> {
        self.inits.fetch_add(1, Ordering::SeqCst);
        match &self.behaviour {
            Behaviour::Provide(f) => f(ctx),
            Behaviour::Panic => panic!("stub plugin {} panicked on purpose", self.manifest.id),
            Behaviour::Hang => {
                tokio::time::sleep(Duration::from_secs(3600)).await;
                unreachable!("the host times out first")
            }
            Behaviour::Fail(e) => Err(e.clone()),
        }
    }
}

#[async_trait]
impl nexus_api::ServicePlugin for StubPlugin {
    async fn init(&self, ctx: PluginContext) -> PluginResult<Provided> {
        EmulatorPlugin::init(self, ctx).await
    }
}

#[async_trait]
impl nexus_api::SystemPlugin for StubPlugin {
    async fn init(&self, ctx: PluginContext) -> PluginResult<Provided> {
        EmulatorPlugin::init(self, ctx).await
    }
}
