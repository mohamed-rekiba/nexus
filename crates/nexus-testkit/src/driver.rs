//! [`ThemeDriver`]: a test drives a shown theme as a player does, through the accessible labels
//! and values its elements carry, keys, clicks and the passing of time, and never through the
//! view model's globals or the theme's callbacks.

use std::time::Duration;

use i_slint_backend_testing::ElementHandle;
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint_interpreter::{ComponentHandle, ComponentInstance};

/// Drives a shown theme on Slint's testing backend. A test host gives the three hooks; the
/// rest is the same for every host.
///
/// The steps that act (`press`, `navigate`, `click`, `type_text`) carry out what the theme
/// asked for, as the app would, and then [`settle`](ThemeDriver::settle).
pub trait ThemeDriver {
    /// The shown theme's window.
    fn shown(&self) -> &ComponentInstance;

    /// Shows the view as it is now.
    fn redraw(&mut self);

    /// Carries out what the theme asked for, as the app would.
    fn carry_out(&mut self);

    /// Lets the entrance animations finish: the testing backend's clock moves only when told
    /// to. It moves twice: a step fires the timers due, then runs change handlers, so a timer
    /// that a handler starts fires on the next step.
    fn settle(&mut self) {
        for _ in 0..2 {
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(3));
            self.redraw();
        }
    }

    /// Moves the testing clock on by `by`, then carries out what the theme asked for.
    fn wait(&mut self, by: Duration) {
        i_slint_backend_testing::mock_elapsed_time(by);
        self.redraw();
        self.carry_out();
        self.redraw();
    }

    /// How many shown elements carry the label `label`.
    fn count(&self, label: &str) -> usize {
        ElementHandle::find_by_accessible_label(self.shown(), label).count()
    }

    /// The first shown element labelled `label`. It fails the test when there is none.
    fn first(&self, label: &str) -> ElementHandle {
        ElementHandle::find_by_accessible_label(self.shown(), label).next().unwrap_or_else(|| panic!("no {label}"))
    }

    /// The values of the elements labelled `label`, in order.
    fn values(&self, label: &str) -> Vec<String> {
        let found = ElementHandle::find_by_accessible_label(self.shown(), label);
        found.map(|e| e.accessible_value().unwrap_or_default().to_string()).collect()
    }

    /// The value of the first element labelled `label`.
    fn value(&self, label: &str) -> String {
        self.first(label).accessible_value().unwrap_or_default().to_string()
    }

    /// The description of the first element labelled `label`.
    fn description(&self, label: &str) -> String {
        self.first(label).accessible_description().unwrap_or_default().to_string()
    }

    /// The values of the elements labelled `label` that show as selected.
    fn focused(&self, label: &str) -> Vec<String> {
        let found = ElementHandle::find_by_accessible_label(self.shown(), label);
        found
            .filter(|e| e.accessible_item_selected() == Some(true))
            .map(|e| e.accessible_value().unwrap_or_default().to_string())
            .collect()
    }

    /// Whether an element labelled `label` shows as selected.
    fn lit(&self, label: &str) -> bool {
        ElementHandle::find_by_accessible_label(self.shown(), label).any(|e| e.accessible_item_selected() == Some(true))
    }

    /// Invokes the element labelled `label` whose value is `value`.
    fn press(&mut self, label: &str, value: &str) {
        let found = ElementHandle::find_by_accessible_label(self.shown(), label);
        let target = found
            .into_iter()
            .find(|e| e.accessible_value().as_deref() == Some(value))
            .unwrap_or_else(|| panic!("no {label} {value}"));
        target.invoke_accessible_default_action();
        self.carry_out();
        self.settle();
    }

    /// Invokes the first element labelled `label`.
    fn press_first(&mut self, label: &str) {
        self.first(label).invoke_accessible_default_action();
        self.carry_out();
        self.settle();
    }

    /// A mouse click in the middle of the first element labelled `label`: press, then release,
    /// as a pointer does, and not the element's default action.
    fn click(&mut self, label: &str) {
        let target = self.first(label);
        let (at, size) = (target.absolute_position(), target.size());
        let position = slint::LogicalPosition::new(at.x + size.width / 2.0, at.y + size.height / 2.0);
        let window = self.shown().window();
        window.dispatch_event(WindowEvent::PointerMoved { position });
        window.dispatch_event(WindowEvent::PointerPressed { position, button: PointerEventButton::Left });
        window.dispatch_event(WindowEvent::PointerReleased { position, button: PointerEventButton::Left });
        self.carry_out();
        self.settle();
    }

    /// Presses and releases `key` in the window, and does nothing more.
    fn send_key(&self, key: Key) {
        let text: slint::SharedString = key.into();
        self.shown().window().dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        self.shown().window().dispatch_event(WindowEvent::KeyReleased { text });
    }

    /// A key, as the window gets it, then a redraw, with nothing carried out and no time
    /// passing.
    fn tap(&mut self, key: Key) {
        self.send_key(key);
        self.redraw();
    }

    /// A key, carried out.
    fn navigate(&mut self, key: Key) {
        self.send_key(key);
        self.carry_out();
        self.settle();
    }

    /// Down, one row at a time and at most `most` times, until an element labelled `label`
    /// shows: rows below a sheet's edge are not drawn.
    fn down_to(&mut self, label: &str, most: usize) {
        for _ in 0..most {
            if self.count(label) > 0 {
                return;
            }
            self.navigate(Key::DownArrow);
        }
        panic!("no {label} after {most} rows down");
    }

    /// Settings: chooses the tab called `name`, as a click does. The focus stays on the tabs.
    fn settings_tab(&mut self, name: &str) {
        self.press_first(&format!("settings-tab:{name}"));
    }

    /// Characters typed on a physical keyboard, one key each, and nothing more.
    fn send_text(&self, text: &str) {
        for ch in text.chars() {
            let s: slint::SharedString = ch.to_string().into();
            self.shown().window().dispatch_event(WindowEvent::KeyPressed { text: s.clone() });
            self.shown().window().dispatch_event(WindowEvent::KeyReleased { text: s });
        }
    }

    /// Characters typed on a physical keyboard, one key each, then carried out.
    fn type_text(&mut self, text: &str) {
        self.send_text(text);
        self.carry_out();
        self.settle();
    }
}
