//! [`Programs`]: the programs a plugin may launch, found and run briefly. The same rules as a
//! launch apply: `permissions.process`, the folder grants and their symlink checks.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use nexus_api::{
    Location, MAX_PROGRAM_OUTPUT, MAX_PROGRAM_TIME, Permissions, PluginError, PluginId, PluginResult, Programs,
};

use crate::ports::{ProcessSpawner, SpawnCommand};
use crate::{ConfigService, DirStorage};

pub(crate) struct HostPrograms {
    pub plugin: PluginId,
    pub permissions: Permissions,
    pub config: Arc<ConfigService>,
    pub home: Option<PathBuf>,
    pub platform: Option<nexus_api::Platform>,
    pub data_dir: PathBuf,
    pub spawner: Arc<dyn ProcessSpawner>,
}

impl HostPrograms {
    /// The real path, after the permission check. `None` when it isn't there.
    fn locate(&self, program: &Location) -> PluginResult<Option<PathBuf>> {
        if !self.permissions.allows_program(program) {
            return Err(PluginError::PermissionDenied(format!("{} may not run {program}", self.plugin)));
        }
        let path = match program {
            Location::Data(path) => DirStorage::new(self.data_dir.clone()).resolve(path)?,
            Location::Folder(path) => {
                match crate::folders::resolve(
                    &self.config,
                    self.home.as_deref(),
                    self.platform,
                    &self.permissions,
                    path,
                ) {
                    Ok(path) => path,
                    Err(PluginError::NotFound(_)) => return Ok(None),
                    Err(e) => return Err(e),
                }
            }
            Location::System(name) => return Ok(self.spawner.which(name)),
        };
        Ok(path.is_file().then_some(path))
    }
}

#[async_trait]
impl Programs for HostPrograms {
    async fn find(&self, program: &Location) -> PluginResult<Option<String>> {
        Ok(self.locate(program)?.map(|p| p.display().to_string()))
    }

    async fn output(&self, program: &Location, args: &[String], timeout: Duration) -> PluginResult<Vec<u8>> {
        let path = self.locate(program)?.ok_or_else(|| PluginError::NotFound(format!("{program} isn't there")))?;
        let cwd = path.parent().map(ToOwned::to_owned).unwrap_or_else(|| self.data_dir.clone());
        let timeout = timeout.min(MAX_PROGRAM_TIME);
        let command = SpawnCommand { program: path, args: args.to_vec(), env: Default::default(), cwd, log: None };
        match self.spawner.output(command, timeout, MAX_PROGRAM_OUTPUT).await? {
            Some(mut out) => {
                out.truncate(MAX_PROGRAM_OUTPUT);
                Ok(out)
            }
            None => Err(PluginError::Timeout(timeout.as_millis() as u64)),
        }
    }

    async fn stream(
        &self,
        program: &Location,
        args: &[String],
        elevated: bool,
        line: &(dyn for<'a> Fn(&'a str) + Send + Sync),
        cancelled: &(dyn Fn() -> bool + Send + Sync),
    ) -> PluginResult<i32> {
        if !self.permissions.allows_long_running(program) {
            return Err(PluginError::PermissionDenied(format!("{} may not run {program} long", self.plugin)));
        }
        if elevated && !self.permissions.allows_polkit(program) {
            return Err(PluginError::PermissionDenied(format!("{} may not run {program} through polkit", self.plugin)));
        }
        let path = self.locate(program)?.ok_or_else(|| PluginError::NotFound(format!("{program} isn't there")))?;
        let cwd = path.parent().map(ToOwned::to_owned).unwrap_or_else(|| self.data_dir.clone());
        let (program, args) = if elevated {
            let pkexec = self.spawner.which("pkexec");
            let pkexec = pkexec.ok_or_else(|| PluginError::NotFound("pkexec isn't there".into()))?;
            (pkexec, std::iter::once(path.display().to_string()).chain(args.iter().cloned()).collect())
        } else {
            (path, args.to_vec())
        };
        let command = SpawnCommand { program, args, env: Default::default(), cwd, log: None };
        self.spawner.stream(command, line, cancelled).await
    }
}
