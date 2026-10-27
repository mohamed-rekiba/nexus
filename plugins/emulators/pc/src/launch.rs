//! Play: the file the Starts setting chose, as it is. What the file is decides how it runs, and
//! nothing else: a script as it is (through `sh` without `#!`), an AppImage without FUSE, a
//! Linux or Mac program as it is, and a Windows program off Windows through umu-launcher
//! (Proton) or Wine. With a Resolution of its own, the game runs inside gamescope. On Linux with
//! GameMode installed, the game runs through `gamemoderun`, which tunes the computer for it while
//! it runs.

use nexus_api::serde_json::Value;
use nexus_api::*;

use crate::Pc;
use crate::library::Game;
use crate::manifest::Start;
use crate::probe::Kind;
use crate::settings::{RESOLUTION, resolution};

/// A program as an argument of gamescope: a system program by its name, which gamescope finds
/// on the PATH, and any other by its path.
fn as_arg(program: Location) -> LaunchArg {
    match program {
        Location::System(name) => LaunchArg::Text(name),
        other => LaunchArg::path(other),
    }
}

impl Pc {
    /// How `start` of `game` runs here, with the game's settings `config`.
    async fn spec(&self, game: &Game, start: &Start, config: &Value) -> PluginResult<LaunchSpec> {
        if !start.kind.runs_on(self.platform) {
            return Err(PluginError::Precondition(start.kind.not_here(self.platform)));
        }
        let file = game.dir.join(&start.path)?;
        let folder = match start.path.rsplit_once('/') {
            Some((dir, _)) => game.dir.join(dir)?,
            None => game.dir.clone(),
        };
        let mut spec = LaunchSpec {
            program: Location::Folder(file.clone()),
            args: Vec::new(),
            env: Default::default(),
            cwd: Some(Location::Folder(folder)),
            game_process: None,
        };
        match start.kind {
            // A script without `#!` (some repacks' `start`) runs with sh.
            Kind::Script => {
                let head = self.ctx.folders().read_range(&file, 0, 2).await.unwrap_or_default();
                if head != b"#!" {
                    spec.program = Location::System("sh".into());
                    spec.args = vec![LaunchArg::path(file)];
                }
            }
            // AppImages unpack and run without FUSE, which minimal systems lack.
            Kind::AppImage => {
                spec.env.insert("APPIMAGE_EXTRACT_AND_RUN".into(), "1".into());
            }
            Kind::Windows if start.kind.needs_runner(self.platform) => {
                self.through_runner(game, file, &mut spec).await?
            }
            Kind::Native | Kind::MacApp | Kind::Windows => {}
        }
        // gamescope is a Linux program: a resolution saved elsewhere does nothing.
        let size = config.get(RESOLUTION).and_then(Value::as_str).and_then(resolution::size);
        if self.platform == Platform::Linux
            && let Some((width, height)) = size
        {
            self.inside_gamescope(width, height, &mut spec).await?;
        }
        if self.platform == Platform::Linux {
            self.through_gamemode(&mut spec).await;
        }
        Ok(spec)
    }

    /// Runs `spec` through `gamemoderun` when GameMode is installed. GameMode then raises the
    /// game's priority and the processor's speed while it runs; for a player in the `gamemode`
    /// group, without a password.
    async fn through_gamemode(&self, spec: &mut LaunchSpec) {
        let gamemode = Location::System("gamemoderun".into());
        if !matches!(self.ctx.programs().find(&gamemode).await, Ok(Some(_))) {
            return;
        }
        let program = std::mem::replace(&mut spec.program, gamemode);
        spec.args = std::iter::once(as_arg(program)).chain(std::mem::take(&mut spec.args)).collect();
    }

    /// Runs the Windows program `exe` through umu-launcher or Wine, in the game's own Wine
    /// prefix.
    async fn through_runner(&self, game: &Game, exe: HostPath, spec: &mut LaunchSpec) -> PluginResult<()> {
        let runner = self.runner().await.ok_or_else(|| PluginError::Precondition(self.runner_hint().into()))?;
        spec.program = runner.program;
        spec.args = vec![LaunchArg::path(exe)];
        // Each game in its own Wine prefix, in this plugin's data folder.
        spec.env.insert("WINEPREFIX".into(), LaunchArg::path(RelPath::new(format!("prefixes/{}", game.id))?));
        if runner.umu {
            // umu downloads and updates Proton (GE-Proton) by itself.
            spec.env.insert("GAMEID".into(), "umu-default".into());
            spec.env.insert("PROTONPATH".into(), "GE-Proton".into());
            // With no 32-bit libraries, Proton's runtime can't give a 32-bit game graphics
            // ("radeonsi: driver missing"): Wine's WoW64 runs it on the 64-bit ones.
            if !self.runs_32bit {
                spec.env.insert("PROTON_USE_WOW64".into(), "1".into());
            }
        }
        Ok(())
    }

    /// Runs `spec` inside gamescope at `width`×`height`, which scales it up to fill the screen.
    async fn inside_gamescope(&self, width: u32, height: u32, spec: &mut LaunchSpec) -> PluginResult<()> {
        let gamescope = Location::System("gamescope".into());
        if !matches!(self.ctx.programs().find(&gamescope).await, Ok(Some(_))) {
            return Err(PluginError::Precondition(format!(
                "To run at {width}×{height} and fill the screen, install gamescope, or set its Resolution back to \
                 Your screen in its settings"
            )));
        }
        let program = std::mem::replace(&mut spec.program, gamescope);
        let size = ["-f", "-w", &width.to_string(), "-h", &height.to_string(), "--"].map(LaunchArg::from);
        spec.args = size.into_iter().chain([as_arg(program)]).chain(std::mem::take(&mut spec.args)).collect();
        Ok(())
    }
}

#[async_trait]
impl Launcher for Pc {
    async fn prepare(&self, game: &str, config: &Value) -> PluginResult<LaunchSpec> {
        let game = self.game(game).await?;
        let start = self.start(&game).await;
        self.spec(&game, &start, config).await
    }
}
