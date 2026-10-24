//! What a console emulator gives a game for input: a controller when one is connected, and its
//! keyboard keys otherwise. Each emulator has its own key layout.

use nexus_api::async_trait;
use nexus_api::{ControllerProfile, Controllers, Order, PluginResult, SectionRow, SlotContent, SlotItem};

/// One emulator's input: the controller profile and the keyboard keys, as (button, keys).
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

#[async_trait]
impl Controllers for Controls {
    async fn profiles(&self) -> PluginResult<Vec<ControllerProfile>> {
        Ok(vec![
            ControllerProfile {
                id: "controller".into(),
                name: "Controller".into(),
                device_hint: Some("DualSense, DualShock 4, Xbox and most others".into()),
            },
            ControllerProfile { id: "keyboard".into(), name: "Keyboard".into(), device_hint: None },
        ])
    }
}
