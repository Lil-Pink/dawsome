//! Recording: where a recording's file goes, the clip it becomes, and (in the window) the
//! input device's stream and the writer of what it captures. The master is recorded through
//! the engine's record tap; the input through a ring the device callback fills.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use dawsome_engine::edit::{takes, Edit};
use dawsome_engine::project::{Project, TrackId};
use dawsome_engine::realtime::RecordClock;
use dawsome_engine::time::Ticks;
use dawsome_media::record::{input_ring, InputFeed};
use thiserror::Error;

use crate::audio::{stream_nanos, PlaybackClock};

/// Seconds of input the ring between the device callback and the writer holds.
const INPUT_RING_SECONDS: usize = 4;

/// How long opening the input device may take before it counts as failed.
const OPEN_TIMEOUT: Duration = Duration::from_secs(2);

/// Why the input device cannot be recorded from.
#[derive(Debug, Error)]
pub enum InputError {
    #[error("no audio input device")]
    NoDevice,
    #[error("the input device offers no 32-bit float stream")]
    NoFloat,
    #[error("cannot open the input device: {0}")]
    Device(#[from] cpal::Error),
}

/// The system's default input device, open and running: its callback copies every buffer into
/// the input ring, keeping the frames only while the song plays for a recording, and raises
/// the meter's peak. Dropping it closes the device.
pub struct InputStream {
    _stream: cpal::Stream,
    /// The device reported an error.
    failed: Arc<AtomicBool>,
}

impl InputStream {
    /// Opens the default input device as 32-bit float, at `project_rate` when it offers it,
    /// else at its own default rate; returns it, the writer's half of its ring, and the peak
    /// cell its meter reads (f32 bits, taken with `swap(0)`). The ring is built before the
    /// stream opens, so the callback never allocates.
    ///
    /// While recording, the callback keeps the frames captured from when the song's first frame
    /// (`clock`'s `recording_from`) played, by `playback`, the output's clock; without one it
    /// keeps every buffer from the first after the recording began.
    pub fn open(
        project_rate: u32,
        clock: RecordClock,
        playback: Option<Arc<PlaybackClock>>,
    ) -> Result<(Self, InputFeed, Arc<AtomicU32>), InputError> {
        let device = cpal::default_host()
            .default_input_device()
            .ok_or(InputError::NoDevice)?;
        let default = device.default_input_config()?;
        let float: Vec<_> = device
            .supported_input_configs()?
            .filter(|range| range.sample_format() == SampleFormat::F32)
            .collect();
        let at_project_rate = float
            .iter()
            .find_map(|range| range.try_with_sample_rate(project_rate));
        let supported = at_project_rate
            .or_else(|| (default.sample_format() == SampleFormat::F32).then_some(default))
            .or_else(|| float.first().map(|range| range.with_max_sample_rate()))
            .ok_or(InputError::NoFloat)?;
        let device_channels = usize::from(supported.channels()).max(1);
        let channels = device_channels.min(2);
        let rate = supported.sample_rate();
        let (mut writer, feed, peak) = input_ring(channels, rate, INPUT_RING_SECONDS);
        let failed = Arc::new(AtomicBool::new(false));
        let reporter = Arc::clone(&failed);
        let offset = Arc::new(AtomicI64::new(0));
        let stream_offset = Arc::clone(&offset);
        let song = playback.clone();
        let stream = device.build_input_stream(
            supported.config(),
            move |data: &[f32], info: &cpal::InputCallbackInfo| {
                let keep_from = clock.recording_from().map(|from| {
                    let start = song.as_ref().and_then(|song| song.time_of(from));
                    let capture = stream_nanos(info.timestamp().capture)
                        + i128::from(stream_offset.load(Ordering::Relaxed));
                    first_kept(start, capture, rate, data.len() / device_channels)
                });
                writer.push(data, device_channels, keep_from);
            },
            move |_: cpal::Error| reporter.store(true, Ordering::Relaxed),
            Some(OPEN_TIMEOUT),
        )?;
        if let Some(playback) = &playback {
            // Before `play`: no callback runs until then.
            offset.store(playback.offset_of(stream.now()), Ordering::Relaxed);
        }
        stream.play()?;
        let input = Self {
            _stream: stream,
            failed,
        };
        Ok((input, feed, peak))
    }

    /// Whether the device has reported an error since it opened.
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }
}

/// The first frame of an input buffer of `len` frames at `rate` that goes into a recording,
/// the buffer's first frame captured at `capture` and the song's first frame played at `start`
/// (nanoseconds on one clock): frames captured before the song's first frame played are
/// dropped. Without `start` (no output stream times the song) the whole buffer goes in.
pub fn first_kept(start: Option<i128>, capture: i128, rate: u32, len: usize) -> usize {
    let Some(start) = start else {
        return 0;
    };
    let early = start - capture;
    if early <= 0 {
        return 0;
    }
    // Rounded up: a frame captured just as the song's first frame plays is kept.
    let frames = (early * i128::from(rate) + 999_999_999) / 1_000_000_000;
    usize::try_from(frames).map_or(len, |frames| frames.min(len))
}

/// The first "<project> take N.wav" in `dir` that does not exist, N counted from 1, and that N:
/// an audio take's file, whose channel is "Take N" ([`take_channel_name`]). Why named after the
/// project: the take is a sample a user drags into another project from the Library, where
/// "rec-0001" told nothing of where it came from.
pub fn next_take_path(dir: &Path, project: &str) -> (PathBuf, u32) {
    (1..)
        .map(|number: u32| (dir.join(format!("{project} take {number}.wav")), number))
        .find(|(path, _)| !path.exists())
        .unwrap_or_else(|| (dir.join(format!("{project} take.wav")), 0))
}

/// The channel an audio take's file plays: "Take N".
pub fn take_channel_name(number: u32) -> String {
    format!("Take {number}")
}

/// The N of an audio take's channel name, "Take N"; none for any other name.
pub fn take_number(name: &str) -> Option<u32> {
    let number = name.strip_prefix("Take ")?;
    (!number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| number.parse().ok())
        .flatten()
}

/// Whether an audio channel named `name` playing `sample` is a take's: "Take N" playing the file
/// its take wrote, "<project> take N.wav" (a channel a user named "Take 2", or a
/// dropped "Take 2.wav", is no take). The rack lists those under one folded header, Recordings.
pub fn is_take(name: &str, sample: &Path) -> bool {
    let Some(number) = take_number(name) else {
        return false;
    };
    sample
        .file_stem()
        .is_some_and(|stem| stem.to_string_lossy().ends_with(&format!(" take {number}")))
}

/// The first `rec-NNNN.wav` in `dir` that does not exist, from `rec-0001.wav` on: what
/// `--play-to --record master` writes (gen_p8's known answers).
pub fn next_recording_path(dir: &Path) -> PathBuf {
    (1..)
        .map(|number: u32| dir.join(format!("rec-{number:04}.wav")))
        .find(|path| !path.exists())
        .unwrap_or_else(|| dir.join("rec.wav"))
}

/// The one edit that adds a new file of `frames` frames (a recording made by `--play-to`, or a
/// file dropped or imported), named `sample` as the project names it, as a new audio channel
/// named after the file and one clip of it from `start`, as long as the file: on `track` when
/// given, else on the first track free over the clip, else on a new track added in the same step
/// (`edit::takes::audio_take`). `None` for an empty file (a recording stopped in the count-in),
/// which adds nothing.
pub fn new_audio_clip(
    project: &Project,
    sample: PathBuf,
    start: Ticks,
    frames: u64,
    track: Option<TrackId>,
) -> Option<Edit> {
    let name = sample.file_stem().map_or_else(
        || "recording".to_owned(),
        |stem| stem.to_string_lossy().into_owned(),
    );
    takes::audio_take(project, name, sample, start, frames, track)
}

#[cfg(test)]
mod tests {
    use dawsome_engine::edit;
    use dawsome_engine::project::{ChannelId, ClipId};

    use super::*;
    use crate::audio::{FrameTiming, PlaybackClock};

    // Input alignment with synthetic timestamps: a 48 kHz project whose output plays player
    // frame 0 at 1 s on the playback clock records from output frame 96000 (after a bar of
    // count-in at 120 BPM), so the song's first frame plays at 3 s. Of an input buffer of 480
    // frames at 48 kHz captured from 2.995 s, the first 240 (5 ms) come before it and are
    // dropped; a buffer captured from 3 s keeps all, one wholly before keeps none, and without
    // output timing the whole buffer goes in.
    #[test]
    fn input_captured_before_the_song_played_is_dropped() {
        let clock = PlaybackClock::new(48_000);
        let timing = FrameTiming {
            first: 0,
            delay: 0,
            device_rate: 48_000,
            project_rate: 48_000,
        };
        clock.publish(timing.origin(1_000_000_000, 0));
        let start = clock.time_of(96_000);
        assert_eq!(start, Some(3_000_000_000));
        assert_eq!(first_kept(start, 2_995_000_000, 48_000, 480), 240);
        assert_eq!(first_kept(start, 3_000_000_000, 48_000, 480), 0);
        assert_eq!(first_kept(start, 2_000_000_000, 48_000, 480), 480);
        assert_eq!(first_kept(None, 2_995_000_000, 48_000, 480), 0);
        let (mut writer, mut feed, _) = dawsome_media::record::input_ring(1, 48_000, 1);
        let buffer: Vec<f32> = (0..480).map(|k| k as f32).collect();
        writer.push(&buffer, 1, Some(240));
        let mut kept = Vec::new();
        dawsome_media::record::Feed::read_into(&mut feed, &mut kept);
        assert_eq!(kept, buffer[240..]);
    }

    fn p8_rec() -> Project {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/p8_rec.json"
        );
        Project::load_json(&std::fs::read_to_string(path).expect("reads")).expect("loads")
    }

    // Known answer 8: two bars recorded from tick 0 of p8_rec (track 1 holds the pattern clip
    // over 0..768) become channel 2 "rec-0001" and clip 2 on track 2, 768 ticks long.
    #[test]
    fn a_recording_lands_on_the_first_free_track_as_long_as_its_file() {
        let mut project = p8_rec();
        let edit = new_audio_clip(
            &project,
            "samples/rec-0001.wav".into(),
            Ticks(0),
            176_400,
            None,
        )
        .expect("an edit");
        assert_eq!(
            edit,
            Edit::AddAudioClip {
                channel: ChannelId(2),
                name: "rec-0001".into(),
                sample: "samples/rec-0001.wav".into(),
                clip: ClipId(2),
                track: TrackId(2),
                start: Ticks(0),
                length: Ticks(768),
            }
        );
        edit::apply(&mut project, edit).expect("applies");
        assert!(
            new_audio_clip(&project, "x.wav".into(), Ticks(0), 0, None).is_none(),
            "empty"
        );
    }

    // With every track taken over the clip, a track is added in the same step.
    #[test]
    fn a_recording_with_no_free_track_adds_one() {
        let mut project = p8_rec();
        project.playlist.tracks.retain(|id, _| *id == TrackId(1));
        project.playlist.next_track_id = TrackId(2);
        let edit = new_audio_clip(&project, "r.wav".into(), Ticks(0), 10, None).expect("an edit");
        let Edit::Transaction(edits) = &edit else {
            panic!("a track and a clip: {edit:?}");
        };
        assert!(matches!(edits[0], Edit::AddTrack { id: TrackId(2), .. }));
        edit::apply(&mut project, edit).expect("applies");
        assert_eq!(project.playlist.tracks.len(), 2);
    }

    // Recording v2: takes are named after the project, from 1, the first number not taken;
    // their channels "Take N", which the rack folds, and no other name.
    #[test]
    fn takes_are_named_after_the_project() {
        let dir = std::env::temp_dir().join(format!("dawsome-take-names-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        assert_eq!(
            next_take_path(&dir, "song"),
            (dir.join("song take 1.wav"), 1)
        );
        std::fs::write(dir.join("song take 1.wav"), b"").expect("writes");
        std::fs::write(dir.join("song take 3.wav"), b"").expect("writes");
        assert_eq!(
            next_take_path(&dir, "song"),
            (dir.join("song take 2.wav"), 2)
        );
        assert_eq!(next_take_path(&dir, "Untitled").1, 1);
        assert_eq!(take_channel_name(2), "Take 2");
        for (name, take) in [
            ("Take 1", Some(1)),
            ("Take 12", Some(12)),
            ("Take", None),
            ("Take ", None),
            ("Take 1a", None),
            ("take 1", None),
            ("Kick take 1", None),
        ] {
            assert_eq!(take_number(name), take, "{name}");
        }
        assert!(is_take("Take 12", Path::new("samples/song take 12.wav")));
        assert!(!is_take("Take 12", Path::new("samples/song take 1.wav")));
        assert!(!is_take("Take 2", Path::new("Take 2.wav")));
    }

    #[test]
    fn recordings_take_the_first_unused_number() {
        let dir = std::env::temp_dir().join(format!("dawsome-rec-names-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        assert_eq!(next_recording_path(&dir), dir.join("rec-0001.wav"));
        std::fs::write(dir.join("rec-0001.wav"), b"").expect("writes");
        assert_eq!(next_recording_path(&dir), dir.join("rec-0002.wav"));
    }
}
