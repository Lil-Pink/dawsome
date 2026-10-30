//! Dawsome's media files: decoding the sound files channels play, reading instrument sets,
//! resampling, writing renders as WAV or MP3, the offline render loop, interchange export, and
//! finding a moved project's samples and shipped sounds. The command line and the app share
//! this; the engine never touches a file.

#![forbid(unsafe_code)]

mod band_limit;
pub mod decode;
pub mod edited;
pub mod export;
pub mod latency;
pub mod paths;
pub mod peaks;
pub mod record;
pub mod render;
pub mod resample;
pub mod sample;
pub mod set_files;
pub mod sounds;
pub mod stream;
pub mod stretch;
mod wav_fmt;
pub mod write;

pub use paths::{
    input_files, rebase_samples, shipped_root, sound_file, ShippedRoots, SHIPPED_PREFIX,
};
pub use peaks::Peaks;
pub use render::{
    frames_for_bars, overwritten_input, render, render_on_thread, render_on_thread_then, Rendered,
};
pub use sample::{load_channel_sample, ChannelSample, SampleNotice};
pub use sounds::{
    edited_dir, preview, remove_stale_partials, stretch_dir, write_float_frames, InstrumentReport,
    MediaSounds, Playback,
};
pub use stream::{StreamingSource, Waiting};
pub use write::{BitDepth, Dither, Encoding, Sink, WriteError, MP3_RATES};
