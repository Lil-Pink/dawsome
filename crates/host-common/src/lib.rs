//! What Dawsome's plugin hosts share, whatever the plugin format. Why one crate and not a copy
//! in each host: the process tree a scan child runs in, its timeout, the catalogue's ranks and
//! staleness rules must have one source of truth; two copies would drift.
//!
//! - [`catalogue`]: what a scan found in each plugin file, kept in the user's cache folder, so a
//!   plugin is loaded into Dawsome's own process only after a scan child has loaded it without
//!   crashing.
//! - [`scan`]: the scan's parent half, one child process per plugin file; the child's half
//!   (loading the file) is each host's own. [`PluginSetup`] says where plugins are looked for
//!   and where the catalogue is kept.
//! - [`notes`]: the notes a plugin's audio half sends and their order, whatever event type the
//!   format carries them in.
//! - [`RenderPump`]: the hosts' main-thread work while a render plays.
//! - [`window`]: plugin windows, the top-level windows of ours a plugin's interface sits in (X11
//!   on Linux, Win32 on Windows); how an interface is put into one is each format's own.
//!
//! No plugin ABI lives here: the hosts (`dawsome-host-clap`) depend on this crate, never the
//! other way round.
//!
//! # Unsafe code
//!
//! The boundary is the operating system: the scan child's process tree (`child.rs`: `kill(2)` and
//! `prctl(2)` on Linux, job objects and thread handles on Windows) and the Win32 plugin windows
//! (`window/win32.rs`); the X11 windows use `x11rb`'s safe API and hold none. Every block is one
//! call or one dereference, under a `// SAFETY:` comment naming the handle or pointer it relies on
//! and where that is made live (a handle just created and owned, a structure of ours the call
//! fills, a window of ours). The lints below enforce one operation per block, each with its
//! comment; code for Windows alone is linted only by a Windows clippy run.

#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]
#![deny(clippy::multiple_unsafe_ops_per_block)]

pub mod catalogue;
mod child;
pub mod notes;
mod pump;
pub mod scan;
mod setup;
pub mod window;

pub use catalogue::Catalogue;
pub use pump::{plugin_ids, RenderPump};
pub use setup::PluginSetup;
