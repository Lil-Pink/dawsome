//! Dawsome engine: project model, edits, musical time, render. Pure and deterministic; no device
//! or UI code.
//!
//! Data flows one way: a [`project::Project`] changes only through [`edit::apply`], a
//! [`compile::Compiler`] turns it into an immutable [`plan::PlayPlan`], and the plan moves to a
//! [`realtime::Player`] that renders it. `Player::process` is the only way to render audio, for
//! offline files and live playback alike.
// No unsafe code here, and none may come: the engine's and the command line's correctness is
// a matter of logic alone; the plugin hosts hold the unsafe boundary (see their module docs).
#![forbid(unsafe_code)]

pub mod compile;
pub mod dsp;
pub mod edit;
pub mod input;
pub mod midi_out;
pub mod plan;
pub mod processor;
pub mod project;
pub mod realtime;
mod render;
pub mod sfz;
pub mod smf;
pub mod song;
pub mod source;
pub mod time;
pub mod transport;
