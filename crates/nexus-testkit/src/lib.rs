//! Test support for the launcher and its plugins.
//!
//! - [`FakeHost`]: a real core over a temporary data folder, fake HTTP, fake processes and a
//!   [`FakeClock`].
//! - [`StubPlugin`]: a plugin built from a manifest and test-supplied behaviour.
//! - [`FakeSystem`]: a system plugin over a machine the test controls.
//! - [`contract`]: checks any emulator plugin against the capability contracts.

mod archives;
mod clock;
pub mod contract;
mod fakes;
mod host;
mod network;
mod spans;
mod stub;
mod system;
mod torrents;

pub use archives::FakeArchives;
pub use clock::FakeClock;
pub use fakes::{FIRST_PID, FakeDesktop, FakeHttp, FakeProcessTree, FakeSpawner, Gate, Shown, Signal, SpawnedProcess};
pub use host::{FakeHost, TestCore, TestHost, until};
pub use network::FakeNetwork;
pub use spans::SpanRecorder;
pub use stub::StubPlugin;
pub use system::{FakeMachine, FakeSystem};
pub use torrents::FakeTorrents;

/// `rel` (parts split by `/`) under `root`, as text with this system's separators: what the
/// host passes to programs.
pub fn native(root: impl AsRef<std::path::Path>, rel: &str) -> String {
    rel.split('/').fold(root.as_ref().to_path_buf(), |path, part| path.join(part)).display().to_string()
}
