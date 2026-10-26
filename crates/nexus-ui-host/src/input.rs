//! Gamepad buttons, as the keys themes already handle.
//!
//! A theme that supports gamepads navigates with the arrow keys, Return, Escape and Tab (see
//! the TV theme). The host turns each button into one of those keys, so a theme needs no
//! gamepad code, and a keyboard does the same as a pad.

use slint::platform::Key;

/// A button on a standard gamepad, by position: `South` is A on Xbox and Cross on PlayStation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PadButton {
    Up,
    Down,
    Left,
    Right,
    /// Confirm.
    South,
    /// Back.
    East,
    North,
    West,
    Start,
    Select,
    LeftShoulder,
    RightShoulder,
    /// L2 and R2.
    LeftTrigger,
    RightTrigger,
    /// The PS or Xbox button (Logitech's too).
    Guide,
    /// The PS or Xbox button, held: closes the running game, else the power menu.
    GuideHeld,
}

/// What a button does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PadAction {
    /// Sent to the theme as this key.
    Key(slint::SharedString),
    /// Back to the library, whatever the screen; nothing while a game runs.
    Home,
    /// Closes the running game at once; with none, the power menu: log out, restart Nexus,
    /// restart, shut down.
    Power,
    Nothing,
}

impl PadButton {
    pub fn action(self) -> PadAction {
        let key = |k: Key| PadAction::Key(k.into());
        match self {
            Self::Up => key(Key::UpArrow),
            Self::Down => key(Key::DownArrow),
            Self::Left => key(Key::LeftArrow),
            Self::Right => key(Key::RightArrow),
            Self::South => key(Key::Return),
            // Options: the game's menu.
            Self::Start => key(Key::Menu),
            Self::East => key(Key::Escape),
            // Next section.
            Self::RightShoulder => key(Key::Tab),
            // Create, Share or View: closes the notice.
            Self::Select => key(Key::Delete),
            Self::Guide => PadAction::Home,
            Self::GuideHeld => PadAction::Power,
            // Triangle: search. Square: the trailer. L1: the screen before. L2 and R2: a page.
            Self::North => PadAction::Key("/".into()),
            // Not a letter: in the Library a typed "t" searches, Square still plays the trailer.
            Self::West => key(Key::Insert),
            Self::LeftShoulder => key(Key::Backtab),
            Self::LeftTrigger => key(Key::PageUp),
            Self::RightTrigger => key(Key::PageDown),
        }
    }
}
