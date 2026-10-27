//! The theme host: shows one theme at a time, and swaps themes without losing state.

use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::SystemTime;

use nexus_api::{GameKey, PluginId};
use nexus_core::{Command, Core, Route, SessionCmd};
use nexus_ui_api::ThemeError;
use slint_interpreter::{ComponentDefinition, ComponentHandle, ComponentInstance, Image, SharedString, Value};

use crate::bind::{self, Viewport};
use crate::compile::compile;
use crate::images::{Crop, ImageCache, ImageKey, ImageSize, Window, decode};
use crate::themes::{ResolvedTheme, ThemeSet};

/// The id of the built-in theme every chain ends in.
pub const BASE_THEME: &str = "default";

/// What the theme asked for that the app has to carry out.
#[derive(Clone, Debug, PartialEq)]
pub enum UiRequest {
    /// Run on the core.
    Core(Command),
    /// Swap to another theme. Not done inside the theme's own callback, because the swap
    /// drops the instance that is running it.
    SwitchTheme(PluginId),
    PlaySound(String),
    /// Open a web page in the system's browser: only `https://` addresses get here.
    OpenUrl(String),
    /// Open a game's folder or log with the system's opener. Only paths the core resolved
    /// for a game get here; the theme names a game, never a path.
    OpenPath(std::path::PathBuf),
    /// Close the launcher.
    Quit,
    /// Close the launcher and start it again, for a kept launcher update to go in.
    Restart,
    /// Close the launcher and end the Nexus session, back to the login screen.
    LogOut,
    /// The trailer player: play an `https://` video, pause or go on, seek, close.
    Trailer(TrailerRequest),
    /// Render again: the theme moved a grid, and other covers are near the screen now.
    Render,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TrailerRequest {
    Play(String),
    TogglePause,
    /// Seconds, forward or back.
    Seek(f64),
    Close,
}

/// The trailer playing, as the app's player says.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrailerStatus {
    pub playing: bool,
    /// Still opening, or waiting for the network.
    pub loading: bool,
    /// Seconds.
    pub position: f64,
    pub duration: f64,
}

pub type Requests = Rc<dyn Fn(UiRequest)>;

/// Turns a `Host` callback's arguments into a request, or nothing when they don't parse.
type ToRequest = Box<dyn Fn(&[Value]) -> Option<UiRequest>>;

struct Live {
    resolved: ResolvedTheme,
    // Kept so a dev-mode reload can compare; the instance holds what it needs.
    _definition: ComponentDefinition,
    instance: ComponentInstance,
    options: serde_json::Value,
    stamp: Option<SystemTime>,
    sounds: BTreeMap<String, std::sync::Arc<[u8]>>,
}

pub struct ThemeHost {
    core: Core,
    themes: ThemeSet,
    live: Option<Live>,
    requests: Requests,
    images: ImageCache,
    safe_mode: bool,
    input_mode: Option<String>,
    /// The app can play trailers itself (libmpv is there).
    trailers: bool,
    trailer: Option<TrailerStatus>,
    pad_name: String,
    monitors: Vec<String>,
    /// A normal window for this run, whatever the setting says (`--windowed`).
    windowed: bool,
    /// The size of everything for this run, whatever the setting says (NEXUS_SCALE).
    size: Option<f32>,
    /// Puts the window in or out of full screen; called on every render, so it checks first.
    fullscreen: Fullscreen,
    /// The local time, for `Vm.clock`. The app sets it each minute.
    clock: String,
    /// Home's art is in, or the boot waited long enough: `Vm.art-ready`. Once true, it stays.
    art_ready: bool,
    /// When the core said it was ready, for the art's time limit.
    boot_ready_at: Option<std::time::Instant>,
    /// The launcher's window has the keyboard focus. The app sets it from the window system.
    window_focused: bool,
    /// How many times the power menu was asked for: `Vm.power-menu`, which the theme watches.
    power_asks: i32,
    /// The view the last render bound: an image coming in binds it again, without asking the
    /// core for a new one.
    last_view: Option<nexus_ui_api::AppView>,
    /// The long lists of cards as last shown: a render changes only the cards that changed.
    cards: bind::CardModels,
}

/// The longest the boot screen waits for Home's art once the core is ready.
const ART_LIMIT: std::time::Duration = std::time::Duration::from_secs(5);

/// Puts a window in full screen (`true`) or out of it.
pub type Fullscreen = Box<dyn Fn(&slint::Window, bool)>;

/// Slint's own full screen.
fn slint_fullscreen(window: &slint::Window, on: bool) {
    if window.is_fullscreen() != on {
        window.set_fullscreen(on);
    }
}

fn base() -> PluginId {
    PluginId::new(BASE_THEME).expect("valid id")
}

impl ThemeHost {
    pub async fn new(core: Core, requests: Requests) -> Self {
        let themes = ThemeSet::from_core(&core).await;
        let safe_mode = core.kernel().host().safe_mode;
        Self {
            core,
            themes,
            live: None,
            requests,
            images: ImageCache::default(),
            safe_mode,
            input_mode: None,
            trailers: false,
            trailer: None,
            pad_name: String::new(),
            monitors: Vec::new(),
            windowed: false,
            size: None,
            fullscreen: Box::new(slint_fullscreen),
            clock: String::new(),
            art_ready: false,
            boot_ready_at: None,
            window_focused: true,
            power_asks: 0,
            last_view: None,
            cards: bind::CardModels::default(),
        }
    }

    pub fn themes(&self) -> &ThemeSet {
        &self.themes
    }

    pub fn current(&self) -> Option<&PluginId> {
        self.live.as_ref().map(|l| l.resolved.id())
    }

    pub fn instance(&self) -> Option<&ComponentInstance> {
        self.live.as_ref().map(|l| &l.instance)
    }

    pub fn resolved(&self) -> Option<&ResolvedTheme> {
        self.live.as_ref().map(|l| &l.resolved)
    }

    /// Shows the player's theme, or the default theme if that fails or in safe mode. The default
    /// theme always works; if it doesn't, that's a bug and this returns the error.
    pub fn start(&mut self) -> Result<(), ThemeError> {
        self.start_with(None)
    }

    /// Like [`Self::start`], showing `theme` instead of the saved choice, without saving it.
    pub fn start_with(&mut self, theme: Option<PluginId>) -> Result<(), ThemeError> {
        let saved = theme.or_else(|| self.core.selected_theme());
        let preferred = if self.safe_mode { base() } else { saved.unwrap_or_else(base) };
        if let Err(error) = self.load(&preferred, false) {
            tracing::error!(theme = %preferred, %error, "theme failed; falling back to the default theme");
            if preferred == base() {
                return Err(error);
            }
            self.load(&base(), false)?;
        }
        self.core.set_active_theme(self.current().cloned().expect("loaded"));
        Ok(())
    }

    /// Swaps to `id`. On any failure the current theme stays, and the error says why.
    pub fn switch(&mut self, id: &PluginId) -> Result<(), ThemeError> {
        if self.safe_mode && id != &base() {
            return Err(ThemeError::Platform("safe mode shows the default theme only".into()));
        }
        self.load(id, true)?;
        self.render()
    }

    /// Validate, load assets, build the new view tree, swap, release the old one. With
    /// `save`, the core saves the choice and announces ThemeChanged just before the swap, so
    /// a failure to save leaves the old theme on screen.
    fn load(&mut self, id: &PluginId, save: bool) -> Result<(), ThemeError> {
        let options = self.core.theme_options(id);
        let resolved = self.themes.resolve(id, &options)?;
        let assets = resolved
            .images()?
            .into_iter()
            .map(|(name, bytes)| {
                decode(&bytes, ImageSize::Full, Crop::None)
                    .map(|image| (name.clone(), image))
                    .map_err(|message| ThemeError::Asset { theme: id.clone(), asset: name, message })
            })
            .collect::<Result<BTreeMap<String, Image>, _>>()?;
        let sounds = resolved.sounds()?.into_iter().map(|(name, bytes)| (name, bytes.into())).collect();
        let definition = compile(&resolved)?;
        let platform = |e: slint_interpreter::PlatformError| ThemeError::Platform(e.to_string());
        let instance = match &self.live {
            Some(live) => definition.create_with_existing_window(live.instance.window()).map_err(platform)?,
            None => definition.create().map_err(platform)?,
        };
        self.wire(&instance, id)?;
        let binding = |message: String| ThemeError::Binding { theme: id.clone(), message };
        bind::set_tokens(&instance, &resolved.merged.tokens).map_err(binding)?;
        bind::set_options(&instance, &resolved.merged.option_values).map_err(binding)?;
        for (name, image) in assets {
            bind::set(&instance, "Assets", &name, Value::Image(image)).map_err(binding)?;
        }
        bind::set(&instance, "Assets", "loaded", Value::Bool(true)).map_err(binding)?;
        // A new theme: the view comes fresh from the core, and the lists start again.
        self.last_view = None;
        self.cards.clear();
        self.bind_view(&instance, &resolved, &self.core.view()).map_err(binding)?;
        if save {
            self.core.theme_switched(id.clone()).map_err(|e| ThemeError::Platform(e.to_string()))?;
        }
        instance.show().map_err(platform)?;

        let stamp = resolved.stamp();
        let old = self.live.replace(Live { resolved, _definition: definition, instance, options, stamp, sounds });
        drop(old);
        tracing::info!(theme = %id, "theme shown");
        Ok(())
    }

    fn bind_view(
        &self,
        instance: &ComponentInstance,
        resolved: &ResolvedTheme,
        view: &nexus_ui_api::AppView,
    ) -> Result<(), String> {
        let (width, height, scale) = window_size(instance);
        let merged = &resolved.merged;
        let input_mode = self.input_mode_of(resolved);
        let size = self.size.unwrap_or(view.settings.scale);
        let aspect = if height > 0.0 { width / height } else { 0.0 };
        let breakpoint = merged.breakpoint(width, height, scale);
        let trailer = self.trailer.clone().unwrap_or_default();
        let host = bind::HostVm {
            viewport: Viewport { width, height, scale, size, aspect, breakpoint },
            input_mode: &input_mode,
            clock: &self.clock,
            can_play_trailers: self.trailers,
            pad_name: &self.pad_name,
            monitors: &self.monitors,
            monitor: self.core.prefs().monitor.unwrap_or_default(),
            has_trailer: self.trailer.is_some(),
            trailer: bind::TrailerVm {
                playing: trailer.playing,
                loading: trailer.loading,
                position: trailer.position,
                duration: trailer.duration,
            },
        };
        let values = bind::vm_values(view, &self.images, &self.cards)?;
        for (name, value) in values.into_iter().chain(bind::host_values(&host, &self.images)?) {
            bind::set(instance, "Vm", &name, value)?;
        }
        Ok(())
    }

    /// Shows the current state. Call it after every core event.
    pub fn render(&mut self) -> Result<(), ThemeError> {
        let view = self.core.view();
        self.show(view)
    }

    /// Shows the images that came in since the last render, with the view it bound: nothing
    /// in the core changed, so the core isn't asked for a new one. Before any render, renders.
    pub fn render_images(&mut self) -> Result<(), ThemeError> {
        match self.last_view.take() {
            Some(view) => self.show(view),
            None => self.render(),
        }
    }

    fn show(&mut self, view: nexus_ui_api::AppView) -> Result<(), ThemeError> {
        let Some(live) = &self.live else { return Ok(()) };
        let id = live.resolved.id().clone();
        // The player changed an option: tokens change, layouts don't, so no recompile.
        let options = self.core.theme_options(&id);
        if options != live.options {
            let resolved = self.themes.resolve(&id, &options)?;
            let binding = |message: String| ThemeError::Binding { theme: id.clone(), message };
            bind::set_tokens(&live.instance, &resolved.merged.tokens).map_err(binding)?;
            bind::set_options(&live.instance, &resolved.merged.option_values).map_err(binding)?;
            let live = self.live.as_mut().expect("checked above");
            live.resolved = resolved;
            live.options = options;
        }
        let live = self.live.as_ref().expect("checked above");
        (self.fullscreen)(live.instance.window(), self.core.prefs().fullscreen && !self.windowed);
        self.images.begin_render();
        let bound = self
            .bind_view(&live.instance, &live.resolved, &view)
            .map_err(|message| ThemeError::Binding { theme: id.clone(), message });
        self.images.end_render();
        // As the old launcher's boot: Home shows once what it shows is loaded.
        if !self.art_ready && self.core.booted() {
            let since = *self.boot_ready_at.get_or_insert_with(std::time::Instant::now);
            self.art_ready = self.images.settled() || since.elapsed() >= ART_LIMIT;
        }
        live.instance
            .set_global_property("Vm", "art-ready", Value::Bool(self.art_ready))
            .map_err(|e| ThemeError::Binding { theme: id.clone(), message: e.to_string() })?;
        self.last_view = Some(view);
        bound
    }

    /// How the window goes full screen, in place of Slint's own. On macOS the app keeps the
    /// window in the current Space, so it can come back in front of a game that closed.
    pub fn set_fullscreen_with(&mut self, fullscreen: Fullscreen) {
        self.fullscreen = fullscreen;
    }

    /// The local time the clock shows, for example "9:38 PM". It shows at the next render.
    pub fn set_clock(&mut self, clock: String) {
        self.clock = clock;
    }

    /// Keeps the window a normal one for this run, whatever the setting says.
    pub fn set_windowed(&mut self, windowed: bool) {
        self.windowed = windowed;
    }

    /// Draws everything at `size` for this run, whatever the setting says. It goes to the
    /// nearest end of 0.8 to 1.4.
    pub fn set_size(&mut self, size: f32) {
        self.size = size.is_finite().then(|| size.clamp(nexus_core::MIN_SCALE, nexus_core::MAX_SCALE));
    }

    /// The app can play trailers in the window; without it the theme opens them in the browser.
    pub fn set_trailers_available(&mut self, available: bool) {
        self.trailers = available;
    }

    /// The monitors' names, for Settings to choose from.
    pub fn set_monitors(&mut self, monitors: Vec<String>) {
        self.monitors = monitors;
    }

    /// The connected controller's name, or `None` when none is.
    pub fn set_pad_name(&mut self, name: Option<String>) {
        self.pad_name = name.unwrap_or_default();
    }

    /// The clock the theme shows, as the app last set it.
    pub fn clock(&self) -> &str {
        &self.clock
    }

    /// How the player drives the shown theme now, as the theme reads it in `Vm.input-mode`:
    /// `gamepad` once a pad button came in, and the theme's first input mode before.
    pub fn input_mode(&self) -> Option<String> {
        self.live.as_ref().map(|live| self.input_mode_of(&live.resolved))
    }

    fn input_mode_of(&self, resolved: &ResolvedTheme) -> String {
        self.input_mode.clone().unwrap_or_else(|| {
            let first = resolved.merged.input.first().copied().unwrap_or(nexus_ui_api::InputMode::Pointer);
            serde_json::to_value(first).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
        })
    }

    /// The trailer playing, as the theme shows it, or `None` once it closed.
    pub fn trailer(&self) -> Option<&TrailerStatus> {
        self.trailer.as_ref()
    }

    /// The shown theme's breakpoint for its window's size, as the theme reads it in
    /// `Vm.viewport`.
    pub fn breakpoint(&self) -> Option<String> {
        let live = self.live.as_ref()?;
        let (width, height, scale) = window_size(&live.instance);
        Some(live.resolved.merged.breakpoint(width, height, scale))
    }

    /// The trailer playing, or `None` once it closed.
    pub fn set_trailer(&mut self, status: Option<TrailerStatus>) {
        self.trailer = status;
    }

    /// A new frame of the trailer, shown at once without binding the whole view.
    pub fn show_trailer_frame(&mut self, pixels: slint::SharedPixelBuffer<slint::Rgba8Pixel>) {
        if let Some(live) = &self.live {
            let _ = live.instance.set_global_property(
                "Vm",
                "trailer-frame",
                Value::Image(slint::Image::from_rgba8(pixels)),
            );
        }
    }

    /// Whether the launcher's window has the keyboard focus. The pad reads controllers
    /// whatever window is in front, and a desktop that can't name the window in front (Wayland)
    /// still tells each window when it gains or loses the focus.
    pub fn set_window_focused(&mut self, focused: bool) {
        self.window_focused = focused;
    }

    /// The player used another input device.
    pub fn set_input_mode(&mut self, mode: &str) {
        self.input_mode = Some(mode.to_owned());
    }

    /// A gamepad button was pressed: the theme gets it as a key press (see
    /// [`crate::PadButton::action`]) while `Vm.pad-key` is true, and `Vm.input-mode` becomes `gamepad`.
    ///
    /// While a game's window is in front, or the launcher's window doesn't have the focus, the
    /// pad is the other window's: only the PS button held reaches the launcher, to close the
    /// running game at once (with no game, it opens the power menu). A tap while a game runs
    /// does nothing.
    pub fn pad(&mut self, button: crate::PadButton) -> Result<(), ThemeError> {
        self.pad_press(button, false)
    }

    /// A held gamepad button repeated: as [`ThemeHost::pad`], with the key press marked as a
    /// repeat, so a theme can ignore it (a held Cross doesn't start a game twice).
    pub fn pad_repeated(&mut self, button: crate::PadButton) -> Result<(), ThemeError> {
        self.pad_press(button, true)
    }

    fn pad_press(&mut self, button: crate::PadButton, repeat: bool) -> Result<(), ThemeError> {
        let action = button.action();
        let reaches = matches!(action, crate::PadAction::Home | crate::PadAction::Power);
        if !reaches && (self.core.game_in_front() || !self.window_focused) {
            return Ok(());
        }
        if self.input_mode.as_deref() != Some("gamepad") {
            self.set_input_mode("gamepad");
            self.render()?;
        }
        match action {
            crate::PadAction::Key(text) => {
                if let Some(live) = &self.live {
                    let window = live.instance.window();
                    let pressed = if repeat {
                        slint::platform::WindowEvent::KeyPressRepeated { text: text.clone() }
                    } else {
                        slint::platform::WindowEvent::KeyPressed { text: text.clone() }
                    };
                    // `Vm.pad-key` is true for this key only: the theme tells it from a keyboard's.
                    let _ = live.instance.set_global_property("Vm", "pad-key", Value::Bool(true));
                    window.dispatch_event(pressed);
                    window.dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
                    let _ = live.instance.set_global_property("Vm", "pad-key", Value::Bool(false));
                }
            }
            // While a game runs, a tap of the PS button does nothing: a hold closes the game.
            crate::PadAction::Home if self.core.game_running() => {}
            crate::PadAction::Home => (self.requests)(UiRequest::Core(Command::Navigate(Route::Home))),
            // While a game runs, the PS button held closes it at once.
            crate::PadAction::Power if self.core.game_running() => {
                for session in self.core.running_sessions() {
                    (self.requests)(UiRequest::Core(Command::Session(SessionCmd::Stop(session))));
                }
            }
            crate::PadAction::Power => {
                self.power_asks += 1;
                if let Some(live) = &self.live {
                    let _ =
                        live.instance.set_global_property("Vm", "power-menu", Value::Number(self.power_asks.into()));
                }
            }
            crate::PadAction::Nothing => {}
        }
        Ok(())
    }

    /// The bytes of the current theme's sound `name`, to play. `None` when the theme has no
    /// such sound, or the player turned sounds off.
    pub fn sound(&self, name: &str) -> Option<std::sync::Arc<[u8]>> {
        if !self.core.prefs().sounds {
            return None;
        }
        self.live.as_ref()?.sounds.get(name).cloned()
    }

    /// Images the last render asked for and doesn't have yet, on-screen ones first.
    pub fn wanted_images(&mut self) -> Vec<ImageKey> {
        self.images.take_wanted()
    }

    /// The images the last render still wants: a loader drops a load that left it.
    pub fn still_wanted(&self) -> crate::StillWanted {
        self.images.still_wanted()
    }

    /// An image made ready on a worker thread (see [`crate::prepare`]); `None` when the
    /// loader dropped it because it left the screen.
    pub fn add_pixels(
        &mut self,
        key: ImageKey,
        pixels: Option<Result<slint_interpreter::SharedPixelBuffer<slint_interpreter::Rgba8Pixel>, String>>,
    ) {
        self.images.insert_pixels(key, pixels);
    }

    /// The images renders asked to load that it still keeps track of: in memory, failed or
    /// loading.
    pub fn images_asked(&self) -> Vec<ImageKey> {
        self.images.asked()
    }

    /// How many images are in memory.
    pub fn images_loaded(&self) -> usize {
        self.images.loaded_count()
    }

    /// The boot screen still waits for Home's art.
    pub fn waiting_for_art(&self) -> bool {
        !self.art_ready
    }

    /// Dev mode: recompiles and swaps the current theme if a file in one of its folders
    /// changed. A broken edit keeps the last good version on screen.
    pub fn reload_if_changed(&mut self) -> Result<bool, ThemeError> {
        let Some(live) = &self.live else { return Ok(false) };
        let id = live.resolved.id().clone();
        if live.resolved.stamp() == live.stamp {
            return Ok(false);
        }
        let result = self.load(&id, false);
        if result.is_err()
            && let Some(live) = self.live.as_mut()
        {
            // Don't retry the same broken files on every tick.
            live.stamp = live.resolved.stamp();
        }
        result.map(|()| true)
    }

    /// Connects the theme's `Host` callbacks to requests: `command` to the core, the others to
    /// the app.
    fn wire(&self, instance: &ComponentInstance, theme: &PluginId) -> Result<(), ThemeError> {
        let callback = |name: &str, f: ToRequest| {
            let requests = self.requests.clone();
            instance
                .set_global_callback("Host", name, move |args| {
                    if let Some(request) = f(args) {
                        requests(request);
                    }
                    Value::Void
                })
                .map_err(|e| ThemeError::Binding { theme: theme.clone(), message: format!("Host.{name}: {e}") })
        };
        callback(
            "command",
            Box::new(move |a| match command(&arg(a, 0), &arg(a, 1)) {
                Ok(command) => Some(UiRequest::Core(command)),
                Err(error) => {
                    tracing::warn!(name = %arg(a, 0), args = %arg(a, 1), %error, "the theme sent a command that doesn't read");
                    None
                }
            }),
        )?;
        // A grid's cards on screen: the covers near them load.
        let windows = self.images.windows();
        callback(
            "grid-window",
            Box::new(move |a| {
                let window = Window { first: int(a, 1), last: int(a, 2) };
                let changed = windows.borrow_mut().insert(arg(a, 0), window) != Some(window);
                changed.then_some(UiRequest::Render)
            }),
        )?;
        callback("switch-theme", Box::new(move |a| PluginId::new(arg(a, 0)).ok().map(UiRequest::SwitchTheme)))?;
        callback("play-sound", Box::new(move |a| Some(UiRequest::PlaySound(arg(a, 0)))))?;
        callback("quit", Box::new(move |_| Some(UiRequest::Quit)))?;
        callback("restart", Box::new(move |_| Some(UiRequest::Restart)))?;
        callback("log-out", Box::new(move |_| Some(UiRequest::LogOut)))?;
        let trailer = |r: TrailerRequest| Some(UiRequest::Trailer(r));
        callback(
            "play-trailer",
            Box::new(move |a| {
                Some(arg(a, 0)).filter(|u| u.starts_with("https://")).and_then(|u| trailer(TrailerRequest::Play(u)))
            }),
        )?;
        callback("pause-trailer", Box::new(move |_| trailer(TrailerRequest::TogglePause)))?;
        callback(
            "seek-trailer",
            Box::new(move |a| match a.first() {
                Some(Value::Number(seconds)) => trailer(TrailerRequest::Seek(*seconds)),
                _ => None,
            }),
        )?;
        callback("close-trailer", Box::new(move |_| trailer(TrailerRequest::Close)))?;
        let folders = self.core.clone();
        callback(
            "open-game-folder",
            Box::new(move |a| game(a, 0).and_then(|g| folders.game_folder(&g)).map(UiRequest::OpenPath)),
        )?;
        let downloads = self.core.clone();
        callback(
            "open-download-folder",
            Box::new(move |a| downloads.download_folder(&arg(a, 0)).map(UiRequest::OpenPath)),
        )?;
        let logs = self.core.clone();
        callback(
            "open-game-log",
            Box::new(move |a| game(a, 0).and_then(|g| logs.game_log(&g)).map(UiRequest::OpenPath)),
        )?;
        callback(
            "open-url",
            Box::new(move |a| Some(arg(a, 0)).filter(|u| u.starts_with("https://")).map(UiRequest::OpenUrl)),
        )?;
        Ok(())
    }
}

/// The command a theme sent with `Host.command`: the name of a [`Command`] variant, and its
/// payload as JSON, empty for none.
fn command(name: &str, args: &str) -> Result<Command, String> {
    let json = if args.trim().is_empty() {
        serde_json::json!(name)
    } else {
        let payload: serde_json::Value =
            serde_json::from_str(args).map_err(|e| format!("the args aren't JSON: {e}"))?;
        serde_json::json!({ name: payload })
    };
    serde_json::from_value(json).map_err(|e| e.to_string())
}

fn arg(args: &[Value], i: usize) -> String {
    match args.get(i) {
        Some(Value::String(s)) => s.to_string(),
        _ => String::new(),
    }
}

fn int(args: &[Value], i: usize) -> usize {
    match args.get(i) {
        Some(Value::Number(n)) if *n >= 0.0 => *n as usize,
        _ => 0,
    }
}

fn game(args: &[Value], i: usize) -> Option<GameKey> {
    arg(args, i).parse().ok()
}

/// The window's logical width and height, and its scale factor.
fn window_size(instance: &ComponentInstance) -> (f32, f32, f32) {
    let size = instance.window().size();
    let scale = instance.window().scale_factor();
    (size.width as f32 / scale, size.height as f32 / scale, scale)
}

/// For tests: the value of a `Vm` property on the live instance, as text when it is text.
pub fn vm_text(instance: &ComponentInstance, name: &str) -> Option<String> {
    match instance.get_global_property("Vm", name).ok()? {
        Value::String(s) => Some(SharedString::to_string(&s)),
        other => Some(format!("{other:?}")),
    }
}

#[cfg(test)]
mod tests {
    use nexus_core::{Command, GridFilter, LibraryCmd, SessionCmd, Sort, SystemChoice, TorrentAction, TorrentCmd};

    use super::command;

    #[test]
    fn a_command_reads_by_its_name_and_its_json_args() {
        assert_eq!(command("Back", ""), Ok(Command::Back));
        assert_eq!(command("LibrarySystem", "null"), Ok(Command::LibrarySystem(None)));
        // The chips' ids and the sorts' names, as the default theme sends them.
        let ps5 = Some(SystemChoice::System("PS5".into()));
        assert_eq!(command("LibrarySystem", "\"PS5\""), Ok(Command::LibrarySystem(ps5)));
        let ingame = Some(SystemChoice::Filter(GridFilter::InGame));
        assert_eq!(command("CatalogSystem", "\"ingame\""), Ok(Command::CatalogSystem(ingame)));
        assert_eq!(command("CatalogSort", "\"played\""), Ok(Command::CatalogSort(Sort::Played)));
        assert_eq!(command("Library", "\"Refresh\""), Ok(Command::Library(LibraryCmd::Refresh)));
        let key: nexus_api::GameKey = "emu:astro".parse().unwrap();
        assert_eq!(command("Session", r#"{"Launch":"emu:astro"}"#), Ok(Command::Session(SessionCmd::Launch(key))));
        assert_eq!(
            command("Torrent", r#"{"Act":{"id":"abc","action":"StopSeeding"}}"#),
            Ok(Command::Torrent(TorrentCmd::Act { id: "abc".into(), action: TorrentAction::StopSeeding }))
        );
    }

    #[test]
    fn a_command_that_doesn_t_read_is_refused() {
        assert!(command("Launch", "").is_err(), "no such command");
        assert!(command("Select", "emu:astro").is_err(), "the args aren't JSON");
        assert!(command("Select", "3").is_err(), "the args are of the wrong type");
    }

    #[test]
    fn a_blank_folder_is_the_one_the_sheet_offers() {
        let read = command("Torrent", r#"{"Download":{"offer":"emu:astro","release":"r","folder":"  "}}"#);
        assert!(matches!(read, Ok(Command::Torrent(TorrentCmd::Download { folder: None, .. }))), "{read:?}");
    }
}
