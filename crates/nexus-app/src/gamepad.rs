//! Controllers, read with `gilrs` on a thread of their own. DualSense, DualShock 4, Xbox and
//! most pads with a standard mapping work. The d-pad and the left stick move; a held button or
//! stick repeats after 380 ms, then every 85 ms. Each press goes to the UI thread as a
//! [`PadButton`], marked when it is a repeat. The PS or Xbox button doesn't repeat: a tap is
//! Home (nothing while a game runs), and held for [`GUIDE_HOLD`] it closes the running game, or with none opens the power
//! menu.

use std::time::{Duration, Instant};

use gilrs::{Axis, Button, EventType, Gilrs};
use nexus_ui_host::PadButton;

fn button(b: Button) -> Option<PadButton> {
    Some(match b {
        Button::DPadUp => PadButton::Up,
        Button::DPadDown => PadButton::Down,
        Button::DPadLeft => PadButton::Left,
        Button::DPadRight => PadButton::Right,
        Button::South => PadButton::South,
        Button::East => PadButton::East,
        Button::North => PadButton::North,
        Button::West => PadButton::West,
        Button::Start => PadButton::Start,
        Button::Select => PadButton::Select,
        Button::LeftTrigger => PadButton::LeftShoulder,
        Button::LeftTrigger2 => PadButton::LeftTrigger,
        Button::RightTrigger2 => PadButton::RightTrigger,
        Button::RightTrigger => PadButton::RightShoulder,
        Button::Mode => PadButton::Guide,
        _ => return None,
    })
}

/// How long the PS or Xbox button is held to close the game or open the power menu.
pub const GUIDE_HOLD: Duration = Duration::from_millis(600);

/// The PS or Xbox button: a tap or a hold. Nothing goes out when it goes down, since a tap and
/// a hold start the same way: a release before [`GUIDE_HOLD`] is the tap, [`PadButton::Guide`];
/// still down at [`GUIDE_HOLD`] is the hold, [`PadButton::GuideHeld`], once.
#[derive(Default)]
pub struct GuideHold {
    down: Option<Instant>,
    held: bool,
}

impl GuideHold {
    pub fn press(&mut self, now: Instant) {
        if self.down.is_none() {
            self.down = Some(now);
            self.held = false;
        }
    }

    /// The tap, when it went up before it was a hold.
    pub fn release(&mut self) -> Option<PadButton> {
        let tap = self.down.take().is_some() && !self.held;
        tap.then_some(PadButton::Guide)
    }

    /// The hold, once, when it has been down [`GUIDE_HOLD`] at `now`.
    pub fn due(&mut self, now: Instant) -> Option<PadButton> {
        let down = self.down?;
        (!self.held && now >= down + GUIDE_HOLD).then(|| {
            self.held = true;
            PadButton::GuideHeld
        })
    }

    /// How long until the hold is due, while it can still come.
    pub fn wait(&self, now: Instant) -> Option<Duration> {
        self.down.filter(|_| !self.held).map(|down| (down + GUIDE_HOLD).saturating_duration_since(now))
    }

    /// A controller came or went: nothing is down.
    pub fn clear(&mut self) {
        self.down = None;
    }
}

/// How far a stick goes before it counts as a press, and how far back before it lets go.
const PUSH: f32 = 0.6;
const RELEASE: f32 = 0.3;

/// The left stick as a d-pad: one press each time it is pushed past [`PUSH`] in a direction.
#[derive(Default)]
pub struct Stick {
    x: i8,
    y: i8,
}

impl Stick {
    /// The press this move makes, if any. `y` is up when positive, as gilrs reports it.
    pub fn moved(&mut self, axis: Axis, value: f32) -> Option<PadButton> {
        let (held, minus, plus) = match axis {
            Axis::LeftStickX => (&mut self.x, PadButton::Left, PadButton::Right),
            Axis::LeftStickY => (&mut self.y, PadButton::Down, PadButton::Up),
            _ => return None,
        };
        let direction = if value >= PUSH {
            1
        } else if value <= -PUSH {
            -1
        } else if value.abs() < RELEASE {
            0
        } else {
            *held
        };
        let pressed = direction != 0 && direction != *held;
        *held = direction;
        pressed.then_some(if direction > 0 { plus } else { minus })
    }

    /// The direction the stick holds on this axis, if any.
    pub fn held(&self, axis: Axis) -> Option<PadButton> {
        let (held, minus, plus) = match axis {
            Axis::LeftStickX => (self.x, PadButton::Left, PadButton::Right),
            Axis::LeftStickY => (self.y, PadButton::Down, PadButton::Up),
            _ => return None,
        };
        match held {
            1 => Some(plus),
            -1 => Some(minus),
            _ => None,
        }
    }
}

/// How long a button or the stick is held before it repeats, and then how often.
const FIRST_REPEAT: Duration = Duration::from_millis(380);
const NEXT_REPEAT: Duration = Duration::from_millis(85);
/// The longest the reader waits for an event before it looks at the held buttons again.
const LONGEST_WAIT: Duration = Duration::from_millis(50);

/// The buttons and stick directions held now, each with the time of its next repeat.
#[derive(Default)]
pub struct Repeater {
    held: Vec<(PadButton, Instant)>,
}

impl Repeater {
    /// `button` went down at `now`. A button already held keeps its time.
    pub fn press(&mut self, button: PadButton, now: Instant) {
        if !self.held.iter().any(|(held, _)| *held == button) {
            self.held.push((button, now + FIRST_REPEAT));
        }
    }

    /// `button` went up.
    pub fn release(&mut self, button: PadButton) {
        self.held.retain(|(held, _)| *held != button);
    }

    /// Nothing is held (a controller came or went).
    pub fn clear(&mut self) {
        self.held.clear();
    }

    /// The buttons whose repeat is due at `now`; each is next due [`NEXT_REPEAT`] later.
    pub fn due(&mut self, now: Instant) -> Vec<PadButton> {
        let mut due = Vec::new();
        for (button, next) in &mut self.held {
            if now >= *next {
                *next = now + NEXT_REPEAT;
                due.push(*button);
            }
        }
        due
    }

    /// How long the reader can wait for an event before the next repeat is due: at least 1 ms,
    /// at most [`LONGEST_WAIT`].
    pub fn wait(&self, now: Instant) -> Duration {
        self.held
            .iter()
            .map(|(_, next)| next.saturating_duration_since(now))
            .min()
            .unwrap_or(LONGEST_WAIT)
            .clamp(Duration::from_millis(1), LONGEST_WAIT)
    }
}

/// Starts reading controllers. `press` and `named` run on the reader's thread: `press` gets each
/// press and whether it is a held button's repeat; `named` gets the first connected controller's
/// name, or `None`, at start and whenever one comes or goes. Without controller support on this
/// system, logs why and returns.
/// Keeps SIGCHLD off this thread and the reader thread gilrs starts from it. Each program the
/// launcher runs ends with one, and on Linux it broke gilrs's wait with an error in the log
/// every time; tokio still gets it on its own threads.
#[allow(unsafe_code)] // Three system calls on a signal set this function owns; only this thread's mask changes.
fn ignore_child_exits() {
    #[cfg(unix)]
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGCHLD);
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
    }
}

pub fn spawn(press: impl Fn(PadButton, bool) + Send + 'static, named: impl Fn(Option<String>) + Send + 'static) {
    let started = std::thread::Builder::new().name("gamepad".into()).spawn(move || {
        ignore_child_exits();
        let mut gilrs = match Gilrs::new() {
            Ok(gilrs) => gilrs,
            Err(error) => {
                tracing::warn!(%error, "no controller support");
                return;
            }
        };
        for (_, pad) in gilrs.gamepads() {
            tracing::info!(name = pad.name(), "controller connected");
        }
        let first = |gilrs: &Gilrs| gilrs.gamepads().next().map(|(_, pad)| pad.name().to_owned());
        named(first(&gilrs));
        let mut stick = Stick::default();
        let mut held = Repeater::default();
        let mut guide = GuideHold::default();
        loop {
            let now = Instant::now();
            let wait = guide.wait(now).map_or(held.wait(now), |g| g.clamp(Duration::from_millis(1), held.wait(now)));
            if let Some(event) = gilrs.next_event_blocking(Some(wait)) {
                let pressed = match event.event {
                    EventType::ButtonPressed(Button::Mode, _) => {
                        guide.press(Instant::now());
                        None
                    }
                    EventType::ButtonReleased(Button::Mode, _) => guide.release(),
                    EventType::ButtonPressed(b, _) => button(b).inspect(|&b| held.press(b, Instant::now())),
                    EventType::ButtonReleased(b, _) => {
                        if let Some(b) = button(b) {
                            held.release(b);
                        }
                        None
                    }
                    EventType::AxisChanged(axis, value, _) => {
                        let before = stick.held(axis);
                        let pressed = stick.moved(axis, value);
                        if before != stick.held(axis) {
                            if let Some(b) = before {
                                held.release(b);
                            }
                            if let Some(b) = stick.held(axis) {
                                held.press(b, Instant::now());
                            }
                        }
                        pressed
                    }
                    EventType::Connected => {
                        tracing::info!(name = gilrs.gamepad(event.id).name(), "controller connected");
                        held.clear();
                        guide.clear();
                        named(first(&gilrs));
                        None
                    }
                    EventType::Disconnected => {
                        held.clear();
                        guide.clear();
                        named(first(&gilrs));
                        None
                    }
                    _ => None,
                };
                if let Some(pressed) = pressed {
                    tracing::debug!(button = ?pressed, from = ?event.event, "controller press");
                    press(pressed, false);
                }
                gilrs.inc();
            }
            if let Some(hold) = guide.due(Instant::now()) {
                tracing::debug!("controller guide held");
                press(hold, false);
            }
            for repeated in held.due(Instant::now()) {
                tracing::debug!(button = ?repeated, "controller repeat");
                press(repeated, true);
            }
        }
    });
    if let Err(error) = started {
        tracing::warn!(%error, "could not start the controller reader");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_button_repeats_after_380_ms_then_every_85_ms() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let mut held = Repeater::default();
        held.press(PadButton::Right, start);
        assert_eq!(held.wait(start), Duration::from_millis(50), "never more than 50 ms");
        assert_eq!(held.due(at(379)), [], "not yet");
        assert_eq!(held.wait(at(379)), Duration::from_millis(1));
        assert_eq!(held.due(at(380)), [PadButton::Right], "the first repeat");
        assert_eq!(held.due(at(464)), []);
        assert_eq!(held.due(at(465)), [PadButton::Right], "85 ms on");
        assert_eq!(held.due(at(550)), [PadButton::Right]);
        held.release(PadButton::Right);
        assert_eq!(held.due(at(2000)), [], "let go: no more repeats");
        held.press(PadButton::South, at(2000));
        held.press(PadButton::South, at(2100));
        assert_eq!(held.due(at(2380)), [PadButton::South], "a second press doesn't restart the wait");
        held.clear();
        assert_eq!(held.due(at(9000)), [], "a controller went: nothing is held");
    }

    #[test]
    fn a_tap_on_the_guide_button_is_home_and_a_hold_is_the_power_menu_once() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let mut guide = GuideHold::default();
        assert_eq!(guide.wait(start), None, "nothing down: no wait");
        guide.press(start);
        assert_eq!(guide.due(at(599)), None, "not a hold yet");
        assert_eq!(guide.wait(at(100)), Some(Duration::from_millis(500)));
        assert_eq!(guide.release(), Some(PadButton::Guide), "a tap");
        assert_eq!(guide.due(at(2000)), None, "up: nothing comes later");

        guide.press(at(3000));
        assert_eq!(guide.due(at(3600)), Some(PadButton::GuideHeld), "held 600 ms");
        assert_eq!(guide.due(at(4000)), None, "once");
        assert_eq!(guide.wait(at(4000)), None);
        assert_eq!(guide.release(), None, "the hold's release is no tap");

        guide.press(at(5000));
        guide.clear();
        assert_eq!(guide.due(at(6000)), None, "a controller went: nothing is down");
    }

    #[test]
    fn a_held_stick_is_held_like_the_d_pad() {
        let mut stick = Stick::default();
        assert_eq!(stick.held(Axis::LeftStickX), None);
        stick.moved(Axis::LeftStickX, -0.9);
        assert_eq!(stick.held(Axis::LeftStickX), Some(PadButton::Left));
        stick.moved(Axis::LeftStickX, 0.4);
        assert_eq!(stick.held(Axis::LeftStickX), Some(PadButton::Left), "between the two: still held");
        stick.moved(Axis::LeftStickX, 0.0);
        assert_eq!(stick.held(Axis::LeftStickX), None);
        stick.moved(Axis::LeftStickY, 0.9);
        assert_eq!(stick.held(Axis::LeftStickY), Some(PadButton::Up));
        assert_eq!(stick.held(Axis::RightStickY), None);
    }

    #[test]
    fn a_stick_presses_once_per_push_and_lets_go_near_the_middle() {
        let mut stick = Stick::default();
        assert_eq!(stick.moved(Axis::LeftStickX, 0.5), None, "not far enough");
        assert_eq!(stick.moved(Axis::LeftStickX, 0.8), Some(PadButton::Right));
        assert_eq!(stick.moved(Axis::LeftStickX, 1.0), None, "still the same push");
        assert_eq!(stick.moved(Axis::LeftStickX, 0.4), None, "between the two: still held");
        assert_eq!(stick.moved(Axis::LeftStickX, 0.7), None, "it never let go");
        assert_eq!(stick.moved(Axis::LeftStickX, 0.1), None);
        assert_eq!(stick.moved(Axis::LeftStickX, 0.9), Some(PadButton::Right), "a second push");
        assert_eq!(stick.moved(Axis::LeftStickY, 0.9), Some(PadButton::Up));
        assert_eq!(stick.moved(Axis::LeftStickY, -0.9), Some(PadButton::Down), "straight to the other side");
        assert_eq!(stick.moved(Axis::RightStickX, 1.0), None, "only the left stick moves");
    }
}
