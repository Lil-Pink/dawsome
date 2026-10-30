//! Dawsome's CLAP plugin host. The plugin ABI lives only here: the engine sees plugins through
//! its own [`dawsome_engine::processor::ProcessorHost`] port, which [`ClapHost`] implements.
//!
//! - [`scan`]: the out-of-process scan, one child process per plugin file, into the
//!   [`catalogue`], so a plugin that crashes while loading takes only its child down. Both are
//!   `dawsome_host_common`'s, shared with the other plugin formats' hosts and re-exported here;
//!   the child's half, loading a `.clap` file, is this crate's.
//! - [`ClapHost`]: creates plugin instances on the main thread (the UI thread, or the command
//!   line's main thread while a render thread plays), runs their main-thread work, shows their
//!   windows (X11 on Linux, Win32 on Windows), and gives the engine each instance's audio half as
//!   a `Processor`.
//!
//! # Unsafe code
//!
//! `clack-host` makes the CLAP ABI safe; the blocks left are what it keeps unsafe by design:
//! loading a plugin file (`PluginEntry::load`, in a scan child, or here only for a file a scan child
//! loaded and that is unchanged since), handing a plugin our window as its parent (`set_parent`,
//! `Window::from_win32_hwnd`: our window outlives the plugin's interface, see `ClapHost::close`) and
//! `poll(2)` on the descriptors a plugin registered. Each is one operation under a `// SAFETY:`
//! comment naming what keeps it sound, which the lints below enforce.

#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]
#![deny(clippy::multiple_unsafe_ops_per_block)]

pub use dawsome_host_common::catalogue;
mod handlers;
mod host;
mod processor;
pub mod scan;
mod sizing;
#[cfg(test)]
#[path = "../../../tests/support/test_plugins.rs"]
mod test_plugins;
mod window;

pub use catalogue::Catalogue;
pub use dawsome_host_common::{PluginSetup, RenderPump};
pub use host::{ClapHost, FORMAT};

use std::collections::BTreeSet;

use dawsome_engine::project::Project;

/// The CLAP plugin ids `project` names, on plugin channels and in effect slots.
pub fn plugin_ids(project: &Project) -> BTreeSet<String> {
    dawsome_host_common::plugin_ids(project, FORMAT)
}
