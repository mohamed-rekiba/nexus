//! Test support for the launcher and its plugins.
//!
//! - [`FakeHost`]: a real core over a temporary data folder, fake HTTP, fake processes and a
//!   [`FakeClock`].
//! - [`StubPlugin`]: a plugin built from a manifest and test-supplied behaviour.
//! - [`FakeCatalog`], [`FakeStore`] and [`FakeService`]: plugins built from a few choices.
//! - [`FakeSystem`]: a system plugin over a machine the test controls.
//! - [`ThemeDriver`]: drives a shown theme through its accessible labels, keys and clicks.
//! - [`FakePlatform`]: the app shell's platform, which records each call.
//! - [`contract`]: checks any emulator plugin against the capability contracts.

mod archives;
mod catalog;
mod clock;
pub mod contract;
mod driver;
mod fakes;
mod host;
mod network;
mod platform;
mod spans;
mod stub;
mod system;
mod torrents;

pub use archives::FakeArchives;
pub use catalog::{FakeCatalog, FakeService, FakeStore};
pub use clock::FakeClock;
pub use driver::ThemeDriver;
pub use fakes::{
    FIRST_PID, FakeHttp, FakeLoginSession, FakeOpener, FakeProcessTree, FakeSpawner, FakeWindows, Gate, Shown, Signal,
    SpawnedProcess,
};
pub use host::{FakeHost, TestCore, TestHost, until};
pub use network::FakeNetwork;
pub use platform::{FakePlatform, PlatformCall};
pub use spans::SpanRecorder;
pub use stub::StubPlugin;
pub use system::{FakeMachine, FakeSystem};
pub use torrents::FakeTorrents;

/// `rel` (parts split by `/`) under `root`, as text with this system's separators: what the
/// host passes to programs.
pub fn native(root: impl AsRef<std::path::Path>, rel: &str) -> String {
    rel.split('/').fold(root.as_ref().to_path_buf(), |path, part| path.join(part)).display().to_string()
}
