//! A console emulator's keyboard keys, for its panel. Each emulator has its own key layout. The
//! host lists the controllers from the manifest's `[[keymap]]`.

use nexus_api::{Order, SectionRow, SlotContent, SlotItem};

/// One emulator's keyboard keys, as (button, keys).
pub struct Controls {
    pub keys: &'static [(&'static str, &'static str)],
}

impl Controls {
    /// The Emulators screen block that lists the keyboard keys.
    pub fn keyboard_panel(&self) -> SlotItem {
        SlotItem {
            id: "keyboard".into(),
            order: Order::default(),
            content: SlotContent::Section {
                title: "Keyboard controls".into(),
                rows: self
                    .keys
                    .iter()
                    .map(|(what, keys)| SectionRow { label: (*what).into(), value: (*keys).into() })
                    .collect(),
            },
        }
    }
}
