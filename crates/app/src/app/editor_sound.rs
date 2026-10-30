//! The sound the sample editor shows (its view buffer): its channel's file decoded at the
//! project's rate and played through the channel's edit list, held as frames so that one op
//! more is applied where they are ([`dawsome_engine::dsp::edits::apply_in_place`], in the
//! op's range: 16 ms on ten minutes of stereo, where a new buffer costs 240 ms), with its peaks
//! for the waveform drawn at a distance, those of the op's range made again. Anything else
//! (an undo, a redo of more than one op, a revert, another sound) builds it again from the
//! file, on a worker while the window runs, the old one shown meanwhile. The frames are the
//! ones playback hears: the same function makes both, so the waveform is what plays. They are
//! shared with the work Save as new file and Play hand to a worker ([`EditorSound::shared`]),
//! so a long sound is written or copied off the window's thread; an op meanwhile (the window
//! dims them, Undo in the main window does not) copies them first.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use dawsome_engine::dsp::edits::{apply_in_place, zero_crossing_in};
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::project::{SoundEdits, SoundOp};
use dawsome_media::{load_channel_sample, Peaks};

/// A channel's sound as the editor holds it.
#[derive(Debug, Clone)]
pub struct EditorSound {
    /// The file as the project names it, and the list its frames are made through.
    pub file: PathBuf,
    pub edits: SoundEdits,
    /// The frames, side by side: 1 or 2 sides.
    samples: Arc<Vec<f32>>,
    sides: u8,
    pub peaks: Peaks,
    /// The file's own frames at the project's rate, before any op.
    pub file_frames: u64,
    /// Why the file could not be read, when it could not (its frames are then none).
    pub problem: Option<String>,
}

impl EditorSound {
    /// The file the project in `dir` names `file`, decoded at `rate`, through `edits`.
    pub fn build(dir: &Path, file: &Path, rate: u32, edits: &SoundEdits) -> Self {
        let loaded = load_channel_sample(dir, file, rate);
        let problem = loaded.file_rate.is_none().then(|| {
            loaded
                .notices
                .first()
                .map_or_else(|| "it holds no sound".to_owned(), ToString::to_string)
        });
        let sides = loaded.buffer.channels().clamp(1, 2);
        let file_frames = loaded.buffer.frames();
        // The decoded frames are shared; the editor's are its own to change. The decode's copy
        // goes as this one is made.
        let mut samples = loaded.buffer.samples().to_vec();
        drop(loaded);
        for op in &edits.ops {
            apply_in_place(&mut samples, usize::from(sides), op);
        }
        let mut sound = Self {
            file: file.to_path_buf(),
            edits: edits.clone(),
            peaks: Peaks::default(),
            samples: Arc::new(samples),
            sides,
            file_frames,
            problem,
        };
        sound.keep_small();
        sound.peaks = Peaks::of_samples(&sound.samples, sides);
        sound
    }

    /// Whether it is the sound of `file` through `edits`.
    pub fn holds(&self, file: &Path, edits: &SoundEdits) -> bool {
        self.file == file && self.edits == *edits
    }

    /// Whether `edits` is its list and one op more: that op, which [`append`](Self::append)
    /// applies alone.
    pub fn one_more<'a>(&self, file: &Path, edits: &'a SoundEdits) -> Option<&'a SoundOp> {
        let (last, before) = edits.ops.split_last()?;
        (self.file == file && before == self.edits.ops.as_slice()).then_some(last)
    }

    /// Its frames.
    pub fn frames(&self) -> u64 {
        (self.samples.len() / usize::from(self.sides)) as u64
    }

    /// 1 for mono, 2 for stereo.
    pub fn sides(&self) -> u8 {
        self.sides
    }

    /// Its values, side by side.
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    /// Its values, shared: what a worker writes or copies from while the window goes on.
    pub fn shared(&self) -> Arc<Vec<f32>> {
        Arc::clone(&self.samples)
    }

    /// `op` applied to the frames where they are, its range's peaks made again (a crop's or a
    /// delete's from where it starts to the end), and the list one op longer.
    pub fn append(&mut self, op: SoundOp) {
        let (lo, hi) = op.cut(self.frames());
        // Shared with a save or a copy under way, the frames are copied first: that work
        // keeps the sound as it was when it began.
        apply_in_place(
            Arc::make_mut(&mut self.samples),
            usize::from(self.sides),
            &op,
        );
        let length_changes = matches!(op, SoundOp::Crop { .. } | SoundOp::Delete { .. });
        if length_changes {
            self.keep_small();
            let from = if matches!(op, SoundOp::Crop { .. }) {
                0
            } else {
                lo
            };
            self.peaks.refresh(&self.samples, self.sides, from, None);
        } else if lo < hi {
            self.peaks.refresh(&self.samples, self.sides, lo, Some(hi));
        }
        self.edits.ops.push(op);
    }

    /// A copy of frames `[from, to)`, inside the sound: what Play hands the preview.
    pub fn part(&self, from: u64, to: u64) -> SampleBuffer {
        part_of(&self.samples, self.sides, from, to)
    }

    /// Where an edge at `frame` snaps: the nearest zero crossing of the sides summed within
    /// `reach` frames ([`zero_crossing_in`]), else `frame`.
    pub fn snap(&self, frame: u64, reach: u64) -> u64 {
        zero_crossing_in(&self.samples, usize::from(self.sides), frame, reach)
    }

    /// The bytes it holds: its frames (as allocated) and its peaks.
    #[cfg(test)]
    pub fn held_bytes(&self) -> usize {
        let pairs: usize = self.peaks.channels.iter().map(Vec::capacity).sum();
        self.samples.capacity() * std::mem::size_of::<f32>()
            + pairs * std::mem::size_of::<(f32, f32)>()
    }

    /// Gives back what a crop or a delete left unused, once it is more than the frames kept:
    /// ten minutes cropped to a second would hold 230 MB otherwise.
    fn keep_small(&mut self) {
        if self.samples.capacity() > 2 * self.samples.len() {
            Arc::make_mut(&mut self.samples).shrink_to_fit();
        }
    }
}

/// A copy of frames `[from, to)` of `samples` (`sides` side by side), inside them: Play's part,
/// on the window's thread for a short one and on a worker for a long one.
pub fn part_of(samples: &[f32], sides: u8, from: u64, to: u64) -> SampleBuffer {
    let sides = usize::from(sides.clamp(1, 2));
    let frames = (samples.len() / sides) as u64;
    let (from, to) = (from.min(frames) as usize, to.min(frames) as usize);
    let values = samples[from * sides..to.max(from) * sides].to_vec();
    if sides == 1 {
        SampleBuffer::mono(values)
    } else {
        SampleBuffer::stereo(values)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use dawsome_engine::dsp::edits::apply;

    use super::*;

    fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
    }

    fn sound(file: &str) -> SampleBuffer {
        load_channel_sample(&fixtures(), Path::new(file), 44_100).buffer
    }

    // The held sound one op at a time equals the file through the whole list (dsp's `apply`,
    // whose bits gen_p30 states), its peaks those of its frames made anew, for every kind of
    // op, on the stereo pair (gen_p30's pair8) and the 72000-frame ramp: an op appended where
    // the frames are gives the bits of the list from the file.
    #[test]
    fn one_op_more_where_the_frames_are_gives_the_bits_of_the_whole_list() {
        let ops = [
            SoundOp::Crop {
                from: 1000,
                to: 71_000,
            },
            SoundOp::FadeIn { from: 0, to: 4800 },
            SoundOp::Delete {
                from: 10_000,
                to: 20_000,
            },
            SoundOp::Gain {
                from: 0,
                to: 60_000,
                centi_db: -300,
            },
            SoundOp::Silence { from: 300, to: 900 },
            SoundOp::Reverse {
                from: 100,
                to: 59_000,
            },
            SoundOp::FadeOut {
                from: 50_000,
                to: 60_000,
            },
            SoundOp::Normalize {
                from: 0,
                to: 60_000,
                centi_db: -100,
            },
        ];
        for file in ["p30_sound/ramp72000.wav", "p30_sound/pair8.wav"] {
            let path = Path::new(file);
            let mut held = EditorSound::build(&fixtures(), path, 44_100, &SoundEdits::default());
            assert_eq!(held.problem, None);
            for (count, op) in ops.iter().enumerate() {
                held.append(*op);
                let list = SoundEdits {
                    ops: ops[..=count].to_vec(),
                    source: None,
                };
                let whole = apply(&sound(file), &list);
                assert_eq!(held.samples(), whole.samples(), "{file} after {op:?}");
                assert_eq!(held.peaks, Peaks::of(&whole), "{file}: peaks after {op:?}");
                assert!(held.holds(path, &list));
                let built = EditorSound::build(&fixtures(), path, 44_100, &list);
                assert_eq!(built.samples(), whole.samples(), "{file}: built anew");
            }
        }
    }

    // The list one op past the one held names that op; any other list, another file or the
    // same list names none.
    #[test]
    fn a_list_one_op_past_the_one_held_names_the_op() {
        let path = Path::new("p30_sound/ones8.wav");
        let fade = SoundOp::FadeIn { from: 0, to: 4 };
        let held = EditorSound::build(&fixtures(), path, 44_100, &SoundEdits::default());
        let one = SoundEdits {
            ops: vec![fade],
            source: None,
        };
        assert_eq!(held.one_more(path, &one), Some(&fade));
        assert_eq!(held.one_more(Path::new("other.wav"), &one), None);
        assert_eq!(held.one_more(path, &SoundEdits::default()), None);
        let two = one.with(fade);
        assert_eq!(held.one_more(path, &two), None);
        assert_eq!(held.part(2, 5).samples(), [1.0, 1.0, 1.0]);
        assert_eq!(held.part(6, 20).frames(), 2, "inside the sound");
    }

    // A file that is not there: no frames, and why.
    #[test]
    fn a_missing_file_holds_no_frames_and_says_why() {
        let held = EditorSound::build(
            &fixtures(),
            Path::new("p30_sound/none.wav"),
            44_100,
            &SoundEdits::default(),
        );
        assert_eq!(held.frames(), 0);
        assert!(held.problem.is_some());
    }
}
