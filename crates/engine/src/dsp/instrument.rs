//! Multisampled instruments: a set of zones, each one sound with the keys and velocities it
//! answers, its root key, gain, loop and envelope. A note sounds every zone that holds its key
//! and velocity (several at once is layering), each read the way the sampler reads its sound:
//! at a speed from the key, by the sampler's 4-point Hermite interpolation, from the absolute
//! frame elapsed (a live note's read point follows the bend: [`Pitch`]). Inside a loop the four
//! points run as the voice reads: after the loop's end its start, and once the voice has wrapped,
//! before its start its end, so a seam interpolates across the wrap both ways.
//!
//! Every value a voice gives is a function of the frames elapsed since its note began (and the
//! frame its note ended, and the frames its pitch moved on), never of state carried from frame
//! to frame, so any split of a render into blocks gives the same bits. The exponentials (gain in decibels, tuning in cents, the
//! release curve) come from [`crate::dsp::exp2`], which uses only + − × ÷, so they are the
//! same bits on every platform and a Python generator can reproduce them exactly.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use crate::dsp::exp2;
use crate::dsp::laws::{channel_pan_gains, insert_pan, insert_pan_gains};
use crate::dsp::pitch::Pitch;
use crate::dsp::sampler::{interpolate, semitone_speed, SampleBuffer};
use crate::dsp::shape::{ended, Cut, Shape};
use crate::sfz::{LoopMode, Region};

/// One zone: a sound and how a note plays it.
#[derive(Debug, Clone)]
pub struct Zone {
    sample: SampleBuffer,
    keys: (u8, u8),
    velocities: (u8, u8),
    root: u8,
    transpose: i32,
    /// 2^(tune/1200), exactly 1 untuned.
    tune: f64,
    /// The volume as a linear gain.
    gain: f64,
    /// amp_veltrack, percent.
    veltrack: f64,
    /// file rate / project rate for a zone that reads its file's own frames, exactly 1 for one
    /// whose sound is at the project's rate.
    file_speed: f64,
    /// The left and right gains of a mono zone panned off center; `None` plays it centered.
    mono_pan: Option<[f32; 2]>,
    /// A stereo zone's pan, by the balance law; 0 leaves it as it is.
    stereo_pan: f32,
    mode: LoopMode,
    /// The loop, its end inclusive, for a looping mode.
    loop_start: u64,
    loop_end: u64,
    /// The first and the last frame it plays, inclusive.
    first: u64,
    last: u64,
    /// In frames.
    attack: u64,
    release: u64,
    sequence: Option<Sequence>,
}

/// A zone's place in a round robin: its counter, and which of every `length` notes it plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Sequence {
    counter: usize,
    length: u32,
    position: u32,
}

/// Where a zone voice reads on a frame: the position in its sound's frames, whether its loop is
/// in force there (the frame after the loop's end is then its start), and whether it has wrapped
/// (the frame before the loop's start is then its end). For a released sustain loop, whether it
/// had wrapped where it read at note-off.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ReadPoint {
    position: f64,
    looping: bool,
    wrapped: bool,
}

/// A zone's sound as loaded: its frames, at the project's rate unless `at_file_rate` says they
/// are the file's own, and the rate the file was recorded at (`None` for a file that could not
/// be read, which has no frames), by which its loop points and offsets, written in the file's
/// frames, are scaled when its frames were converted.
#[derive(Debug, Clone)]
pub struct ZoneSample {
    pub buffer: SampleBuffer,
    pub file_rate: Option<u32>,
    /// The frames are the file's own, at `file_rate`, not converted: what a loader gives a zone
    /// whose loop is no whole number of frames at the project's rate
    /// ([`FileLoop::reads_file_rate`]), low-passed below half the project's rate when the file's
    /// is above it, frame for frame. The zone then reads them at file_rate / rate times its
    /// speed, its points as written.
    pub at_file_rate: bool,
}

/// An instrument: its zones in the order its file lists them. Cloning shares them.
#[derive(Debug, Clone, Default)]
pub struct Instrument {
    zones: Arc<[Zone]>,
    /// How many round-robin counters its zones use.
    counters: usize,
}

impl Instrument {
    /// The instrument `regions` describe, at the project's `rate`, each region's sound from
    /// `sample` (given the region's sample path). Regions that cannot play (no sound, an offset
    /// past their end) are left out; loops that do not fit play without a loop. Each such
    /// choice is described in the returned list.
    pub fn build(
        regions: &[Region],
        mut sample: impl FnMut(&str) -> ZoneSample,
        rate: u32,
    ) -> (Self, Vec<String>) {
        Self::build_with(regions, |region| sample(&region.sample), rate)
    }

    /// As [`build`](Self::build), each region's sound from `sample` given the region itself: a
    /// loader that converts a file for the loop a region plays ([`FileLoop`]) gives each region
    /// its own conversion.
    pub fn build_with(
        regions: &[Region],
        mut sample: impl FnMut(&Region) -> ZoneSample,
        rate: u32,
    ) -> (Self, Vec<String>) {
        let mut zones = Vec::with_capacity(regions.len());
        let mut problems = Vec::new();
        let mut counters = 0;
        for region in regions {
            let loaded = sample(region);
            match zone(region, loaded, rate, &mut counters) {
                Ok((zone, problem)) => {
                    problems.extend(problem.map(|text| format!("{}: {text}", region.at)));
                    zones.push(zone);
                }
                Err(text) => problems.push(format!("{}: {text}", region.at)),
            }
        }
        let instrument = Self {
            zones: zones.into(),
            counters,
        };
        (instrument, problems)
    }

    /// How many zones it has.
    pub fn zones(&self) -> usize {
        self.zones.len()
    }

    /// Whether `other` shares this one's zones: a voice started on one may go on in the other.
    pub(crate) fn same(&self, other: &Instrument) -> bool {
        Arc::ptr_eq(&self.zones, &other.zones)
    }

    /// Who this instrument is, for a voice to tell it from another.
    pub(crate) fn identity(&self) -> usize {
        Arc::as_ptr(&self.zones).cast::<Zone>() as usize
    }

    /// How many round-robin counters it needs.
    pub(crate) fn counters(&self) -> usize {
        self.counters
    }

    pub(crate) fn zone(&self, index: u32) -> Option<&Zone> {
        self.zones.get(index as usize)
    }

    /// Calls `voice(zone index, speed, gain)` for every zone a note of `key` at `velocity`
    /// (0 to 1) sounds, in file order, counting the note on each round-robin zone it falls in,
    /// on a channel whose sound settings are `shape`: each zone's speed tuned by them and its
    /// velocity gain weighted by them; `None` plays as the set does.
    pub(crate) fn start_shaped(
        &self,
        key: u8,
        velocity: f32,
        counters: &[AtomicU32],
        shape: Option<&Shape>,
        mut voice: impl FnMut(u32, f64, f32),
    ) {
        if velocity.is_nan() || velocity <= 0.0 {
            return;
        }
        let byte = velocity_byte(velocity);
        for (index, zone) in self.zones.iter().enumerate() {
            if !zone.holds(key, byte) {
                continue;
            }
            if let Some(sequence) = zone.sequence {
                let count = counters
                    .get(sequence.counter)
                    .map_or(0, |counter| counter.fetch_add(1, Ordering::Relaxed));
                if count % sequence.length != sequence.position - 1 {
                    continue;
                }
            }
            let Ok(index) = u32::try_from(index) else {
                return;
            };
            match shape {
                None => voice(index, zone.speed(key), zone.gain(byte)),
                Some(shape) => {
                    let speed = zone.speed(key);
                    let speed = if shape.tune == 1.0 {
                        speed
                    } else {
                        speed * shape.tune
                    };
                    let gain = shape.weighted(velocity_gain(byte, zone.veltrack));
                    voice(index, speed, (zone.gain * gain) as f32);
                }
            }
        }
    }
}

/// The zone `region` describes with its sound `loaded`, and a note about a loop it could not
/// keep; `Err` says why it cannot play at all.
fn zone(
    region: &Region,
    loaded: ZoneSample,
    rate: u32,
    counters: &mut usize,
) -> Result<(Zone, Option<String>), String> {
    let frames = loaded.buffer.frames();
    if frames == 0 {
        return Err(format!(
            "{} has no sound; the region is left out",
            region.sample
        ));
    }
    // Frames of a file at another rate, read as they are: its points stay as written, and its
    // speed carries the ratio of the rates.
    let own_rate = match loaded.file_rate {
        Some(file_rate) if loaded.at_file_rate && file_rate != rate && file_rate > 0 => {
            Some(file_rate)
        }
        _ => None,
    };
    let scaled_from = if own_rate.is_some() {
        None
    } else {
        loaded.file_rate
    };
    let at_rate = |frame: u64| at_rate(frame, scaled_from, rate);
    let end_at_rate = |end: u64, from: u64| end_at_rate(end, from, scaled_from, rate);
    let first = at_rate(region.offset);
    let last = region
        .end
        .map_or(frames - 1, |end| end_at_rate(end, first).min(frames - 1));
    if first > last {
        return Err(format!(
            "offset {} is past the sound's last frame; the region is left out",
            region.offset
        ));
    }
    let mut mode = loop_mode(region);
    let loop_start = at_rate(region.loop_start.unwrap_or(0));
    let loop_end = region
        .loop_end
        .map_or(last, |end| end_at_rate(end, loop_start));
    let mut problem = None;
    let loops = matches!(mode, LoopMode::Continuous | LoopMode::Sustain);
    if loops && !(loop_start <= loop_end && loop_end <= last && first <= loop_end) {
        problem = Some(format!(
            "loop {loop_start}..{loop_end} is outside frames {first}..{last}; it plays without a loop"
        ));
        mode = LoopMode::NoLoop;
    }
    let sequence = if region.seq_length > 1 {
        if region.seq_position > region.seq_length {
            return Err(format!(
                "seq_position {} is past seq_length {}; the region never plays",
                region.seq_position, region.seq_length
            ));
        }
        let sequence = Sequence {
            counter: *counters,
            length: region.seq_length,
            position: region.seq_position,
        };
        *counters += 1;
        Some(sequence)
    } else {
        None
    };
    let pan = (region.pan / 100.0) as f32;
    let mono = loaded.buffer.channels() == 1;
    let zone = Zone {
        keys: (region.lokey, region.hikey),
        velocities: (region.lovel, region.hivel),
        root: region.pitch_keycenter,
        transpose: region.transpose,
        tune: if region.tune == 0.0 {
            1.0
        } else {
            exp2(region.tune / 1200.0)
        },
        gain: exp2(region.volume / 20.0 * std::f64::consts::LOG2_10),
        veltrack: region.amp_veltrack,
        file_speed: own_rate.map_or(1.0, |file_rate| f64::from(file_rate) / f64::from(rate)),
        mono_pan: (mono && pan != 0.0).then(|| channel_pan_gains(pan)),
        stereo_pan: if mono { 0.0 } else { pan },
        mode,
        loop_start,
        loop_end,
        first,
        last,
        attack: frames_of(region.ampeg_attack, rate),
        release: frames_of(region.ampeg_release, rate),
        sequence,
        sample: loaded.buffer,
    };
    Ok((zone, problem))
}

/// How `region` plays its loop: its `loop_mode`, else a continuous loop when it gives loop
/// points, else none.
fn loop_mode(region: &Region) -> LoopMode {
    let given = region.loop_start.is_some() || region.loop_end.is_some();
    region.loop_mode.unwrap_or(if given {
        LoopMode::Continuous
    } else {
        LoopMode::NoLoop
    })
}

/// A looping region's loop in its file's own frames, as it is known before the file is read:
/// where the loop starts, and the ends the region names (its `loop_end`, else its `end`, else
/// the file's last frame), so the loop's end can be told once the file's length is.
///
/// What converting the file to another rate needs: a voice reads the loop's start after the
/// loop's last frame, so where that is the file's last frame the conversion must hear the loop
/// go on there, not silence (heard as a tick at every wrap: the edge of the silence bent the
/// last frames, −50 dB on the 808s).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileLoop {
    start: u64,
    loop_end: Option<u64>,
    end: Option<u64>,
    offset: u64,
}

impl FileLoop {
    /// The loop `region` plays, or `None` when it plays none.
    pub fn of(region: &Region) -> Option<Self> {
        matches!(loop_mode(region), LoopMode::Continuous | LoopMode::Sustain).then_some(Self {
            start: region.loop_start.unwrap_or(0),
            loop_end: region.loop_end,
            end: region.end,
            offset: region.offset,
        })
    }

    /// The loop's first and last frame in a file of `frames` frames, when it is one its zone
    /// keeps: it starts at or before its end, at or after the region's first frame, and ends
    /// inside the region.
    pub fn kept(&self, frames: u64) -> Option<(u64, u64)> {
        let last = frames.checked_sub(1)?;
        let region_last = self.end.map_or(last, |end| end.min(last));
        let loop_end = self.loop_end.unwrap_or(region_last);
        let kept = self.start <= loop_end && self.offset <= loop_end && loop_end <= region_last;
        kept.then_some((self.start, loop_end))
    }

    /// The frame a voice reads after the last frame of a file of `frames` frames: the loop's
    /// start, when the loop ends on that frame and is one its zone keeps; `None` when the loop
    /// ends earlier, so the file's own frames follow it.
    pub fn after_last(&self, frames: u64) -> Option<u64> {
        let (start, end) = self.kept(frames)?;
        (end + 1 == frames).then_some(start)
    }

    /// Whether a zone playing this loop from a file of `frames` frames at `file_rate` reads the
    /// file's own frames in a project at `rate`, rather than the file converted to `rate`: when
    /// the rates differ and the loop, one its zone keeps, is no whole number of frames at `rate`
    /// (N·rate/file_rate for a loop of N frames).
    ///
    /// Why: a converted loop has a whole number of frames, the nearest, so a loop that is not
    /// whole slipped by what rounding took off it at every wrap (0.26 to 0.66 frame on FreePats'
    /// 44.1 kHz loops at 48 kHz: a click 4 to 16 times the loop's own second difference, and a
    /// short loop off pitch). Read at the file's rate it wraps every N of the file's frames, as it
    /// does at that rate. A whole loop is converted, as is every other sound.
    pub fn reads_file_rate(&self, frames: u64, file_rate: u32, rate: u32) -> bool {
        if file_rate == rate || file_rate == 0 {
            return false;
        }
        self.kept(frames).is_some_and(|(start, end)| {
            let length = u128::from(end - start) + 1;
            length * u128::from(rate) % u128::from(file_rate) != 0
        })
    }
}

/// A frame of a file recorded at `file_rate` as a frame of the same moment at `rate`, rounded
/// half up; unchanged when the rates are the same or the file's is not known.
fn at_rate(frame: u64, file_rate: Option<u32>, rate: u32) -> u64 {
    match file_rate {
        Some(from) if from != rate && from > 0 => {
            let scaled = (u128::from(frame) * u128::from(rate) * 2 + u128::from(from))
                / (2 * u128::from(from));
            u64::try_from(scaled).unwrap_or(u64::MAX)
        }
        _ => frame,
    }
}

/// An inclusive last frame of a span of a file at `file_rate` (a loop's end, a region's `end`)
/// as one at `rate`, for a span whose first frame is `from` at `rate`: the frame before the one
/// that begins the moment `frame + 1` begins, so a span of n frames is n·rate/file_rate frames
/// long when that is whole. A span that would come out shorter than a frame keeps the frame its
/// end lands on, as scaling the end itself gives.
///
/// Why not scale `frame` itself: it names the start of the span's last frame, so every span came
/// out one frame short above the file's rate (a loop of 2205 frames at 44.1 kHz was 4799 at
/// 96 kHz, not 4800, and ticked once a wrap).
fn end_at_rate(frame: u64, from: u64, file_rate: Option<u32>, rate: u32) -> u64 {
    let after = at_rate(frame.saturating_add(1), file_rate, rate);
    if after > from {
        after - 1
    } else {
        at_rate(frame, file_rate, rate)
    }
}

/// `seconds` as whole frames at `rate`, rounded half away from zero.
fn frames_of(seconds: f64, rate: u32) -> u64 {
    (seconds * f64::from(rate)).round() as u64
}

/// A note's velocity (0 to 1, the project's: a MIDI byte over 128) as a MIDI byte from 1 to
/// 127, as regions' velocity ranges count it.
pub(crate) fn velocity_byte(velocity: f32) -> u8 {
    (f64::from(velocity) * 128.0).round().clamp(1.0, 127.0) as u8
}

/// The gain velocity byte `byte` gives at `veltrack` percent: 1 − t(1 − (b/127)²) for t ≥ 0,
/// |t|(1 − (b/127)²) below.
fn velocity_gain(byte: u8, veltrack: f64) -> f64 {
    let track = veltrack / 100.0;
    let normal = f64::from(byte) / 127.0;
    let curve = normal * normal;
    let lost = track.abs() * (1.0 - curve);
    if track < 0.0 {
        lost
    } else {
        1.0 - lost
    }
}

impl Zone {
    fn holds(&self, key: u8, byte: u8) -> bool {
        (self.keys.0..=self.keys.1).contains(&key)
            && (self.velocities.0..=self.velocities.1).contains(&byte)
    }

    /// The speed a note of `key` reads the sound at: 2^((key − root + transpose)/12), tuned,
    /// times the rates' ratio for a zone on its file's own frames (× 1 changes no bit).
    fn speed(&self, key: u8) -> f64 {
        let semitones = i32::from(key) - i32::from(self.root) + self.transpose;
        semitone_speed(semitones) * self.tune * self.file_speed
    }

    /// The gain of a note at velocity byte `byte`, rounded to f32 once.
    fn gain(&self, byte: u8) -> f32 {
        (self.gain * velocity_gain(byte, self.veltrack)) as f32
    }

    /// Whether its frames go out as a stereo pair (a stereo sound, or a mono one panned).
    pub(crate) fn stereo(&self) -> bool {
        self.sample.channels() == 2 || self.mono_pan.is_some()
    }

    /// A voice's frame `t` frames after its note began, reading at `speed`, its note ended at
    /// frame `off` (none while held): the zone's value and the envelope's level, or `None` once
    /// the voice has ended (played past its sound, or released).
    #[cfg(test)]
    pub(crate) fn frame(&self, t: u64, off: Option<u64>, speed: f64) -> Option<([f32; 2], f32)> {
        self.frame_read(t, off, None, &Pitch::steady(speed))
    }

    /// As [`frame`](Self::frame), reading as `pitch` says, cut at `cut` (a later note on a
    /// channel that cuts itself, [`ended`]): a one-shot hears no note's end, but a cut.
    fn frame_read(
        &self,
        t: u64,
        off: Option<u64>,
        cut: Option<Cut>,
        pitch: &Pitch,
    ) -> Option<([f32; 2], f32)> {
        let off = if self.mode == LoopMode::OneShot {
            None
        } else {
            off
        };
        let level = ended(self.attack, self.release, t, off, cut)?;
        let value = self.read(t, Cut::end(cut, off), pitch)?;
        Some((value, level as f32))
    }

    /// As [`frame`](Self::frame) on a channel whose sound settings are `shape` (`None`: the
    /// zone's own): their attack and release where set in place of the zone's, and the note's
    /// end heard by a zone that does not loop as the channel's gate says
    /// ([`Shape::zone_hears_off`]), which wins over the zone's own one-shot; cut at `cut`, as
    /// [`frame_read`](Self::frame_read) is.
    pub(crate) fn frame_shaped(
        &self,
        t: u64,
        off: Option<u64>,
        cut: Option<Cut>,
        pitch: &Pitch,
        shape: Option<&Shape>,
    ) -> Option<([f32; 2], f32)> {
        let Some(shape) = shape else {
            return self.frame_read(t, off, cut, pitch);
        };
        let heard = match self.mode {
            LoopMode::Continuous | LoopMode::Sustain => true,
            LoopMode::OneShot => shape.zone_hears_off(true),
            LoopMode::NoLoop => shape.zone_hears_off(false),
        };
        let off = if heard { off } else { None };
        let attack = if shape.attack > 0 {
            shape.attack
        } else {
            self.attack
        };
        let release = if shape.release > 0 {
            shape.release
        } else {
            self.release
        };
        let level = ended(attack, release, t, off, cut)?;
        let value = self.read(t, Cut::end(cut, off), pitch)?;
        Some((value, level as f32))
    }

    /// The zone's value `t` frames after its note began, reading as `pitch` says, its note's end
    /// heard at `off`; `None` past its last frame.
    fn read(&self, t: u64, off: Option<u64>, pitch: &Pitch) -> Option<[f32; 2]> {
        let point = self.position(t, off, pitch);
        // Positions are never negative, so the conversion to an integer is the floor.
        let frame = point.position as u64;
        if frame > self.last {
            return None;
        }
        let fraction = point.position - frame as f64;
        let current = self.sample.frame(frame)?;
        let value = if fraction == 0.0 {
            current
        } else {
            let next = self.after(frame, point.looping);
            let frames = [
                self.before(frame, point.wrapped),
                Some(frame),
                next,
                next.and_then(|next| self.after(next, point.looping)),
            ];
            interpolate(
                frames.map(|frame| {
                    frame
                        .and_then(|frame| self.sample.frame(frame))
                        .unwrap_or([0.0; 2])
                }),
                fraction,
            )
        };
        Some(match self.mono_pan {
            Some([left, right]) => [value[0] * left, value[1] * right],
            None if self.stereo_pan != 0.0 => {
                insert_pan(self.stereo_pan, insert_pan_gains(self.stereo_pan), value)
            }
            None => value,
        })
    }

    /// The frame a voice reads after `frame`: the loop's start after its end while the loop is in
    /// force, else the next one up to the last; `None` past the last.
    fn after(&self, frame: u64, looping: bool) -> Option<u64> {
        if looping && frame == self.loop_end {
            Some(self.loop_start)
        } else {
            (frame < self.last).then_some(frame + 1)
        }
    }

    /// The frame a voice read before `frame`: once it has wrapped, the loop's end before the
    /// loop's start and the frame below anywhere else (its first frame no edge any more, even
    /// when it lies inside the loop); before that the frame below, down to its first.
    fn before(&self, frame: u64, wrapped: bool) -> Option<u64> {
        if wrapped && frame == self.loop_start {
            Some(self.loop_end)
        } else if wrapped || frame > self.first {
            frame.checked_sub(1)
        } else {
            None
        }
    }

    /// Where the voice reads at frame `t`, and how its loop stands there.
    fn position(&self, t: u64, off: Option<u64>, pitch: &Pitch) -> ReadPoint {
        let linear = |t: u64| self.first as f64 + pitch.at(t);
        let looped = |position: f64, looping: bool| {
            let (position, wrapped) = self.wrap(position);
            ReadPoint {
                position,
                looping,
                wrapped,
            }
        };
        match self.mode {
            LoopMode::Continuous => looped(linear(t), true),
            LoopMode::Sustain => match off {
                Some(off) if t >= off => {
                    let end = looped(self.first as f64 + pitch.at_end(off), false);
                    ReadPoint {
                        position: end.position + pitch.between(off, t),
                        ..end
                    }
                }
                _ => looped(linear(t), true),
            },
            LoopMode::NoLoop | LoopMode::OneShot => ReadPoint {
                position: linear(t),
                looping: false,
                wrapped: false,
            },
        }
    }

    /// `position` brought back into the loop once it has run past the loop's end, and whether it
    /// had.
    fn wrap(&self, position: f64) -> (f64, bool) {
        let end = (self.loop_end + 1) as f64;
        if position < end {
            return (position, false);
        }
        let start = self.loop_start as f64;
        (start + (position - start) % (end - start), true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sfz::Location;

    fn region(sample: &str) -> Region {
        let text = format!("<region> sample={sample}");
        crate::sfz::parse("t.sfz", &text, &mut |_| Err(String::new()))
            .regions
            .remove(0)
    }

    fn loaded(frames: Vec<f32>) -> ZoneSample {
        ZoneSample {
            buffer: SampleBuffer::from(frames),
            file_rate: Some(44_100),
            at_file_rate: false,
        }
    }

    fn built(region: &Region, frames: Vec<f32>) -> Zone {
        let (instrument, problems) = Instrument::build(
            std::slice::from_ref(region),
            |_| loaded(frames.clone()),
            44_100,
        );
        assert_eq!(problems, Vec::<String>::new());
        instrument.zones[0].clone()
    }

    // Hand-computed: 0.78125 (100/128) is byte 100, 1.0 is 127 at most, the smallest positive
    // velocity is byte 1; at amp_veltrack 100 the gain is (b/127)², at 0 it is 1, at -100
    // 1 − (b/127)².
    #[test]
    fn velocity_bytes_and_gains() {
        assert_eq!(velocity_byte(0.781_25), 100);
        assert_eq!(velocity_byte(1.0), 127);
        assert_eq!(velocity_byte(0.001), 1);
        assert_eq!(velocity_gain(127, 100.0), 1.0);
        assert_eq!(velocity_gain(64, 0.0), 1.0);
        let half = 64.0 / 127.0;
        assert_eq!(velocity_gain(64, 100.0), 1.0 - (1.0 - half * half));
        assert_eq!(velocity_gain(127, -100.0), 0.0);
    }

    // Loop points written for a 22050 Hz file land on the same moments at 44100 Hz: the loop
    // frames 3..=7 (5 frames, the moments 3/22050 up to 8/22050) are 6..=15 (10 frames), the
    // inclusive end scaled as the start of the frame after it, less one.
    #[test]
    fn loop_points_follow_a_resampled_file() {
        let mut ramp = region("r.wav loop_mode=loop_continuous loop_start=3 loop_end=7 offset=1");
        ramp.at = Location {
            file: "t.sfz".to_owned(),
            line: 1,
        };
        let sample = ZoneSample {
            buffer: SampleBuffer::from(vec![0.5; 20]),
            file_rate: Some(22_050),
            at_file_rate: false,
        };
        let (instrument, _) = Instrument::build(&[ramp], |_| sample.clone(), 44_100);
        let zone = &instrument.zones[0];
        assert_eq!((zone.first, zone.loop_start, zone.loop_end), (2, 6, 15));
    }

    fn scaled(sfz: &str, frames: usize, file_rate: u32, rate: u32) -> (u64, u64, u64, u64) {
        let sample = ZoneSample {
            buffer: SampleBuffer::from(vec![0.5; frames]),
            file_rate: Some(file_rate),
            at_file_rate: false,
        };
        let (instrument, problems) = Instrument::build(&[region(sfz)], |_| sample.clone(), rate);
        assert_eq!(problems, Vec::<String>::new());
        let zone = &instrument.zones[0];
        (zone.first, zone.last, zone.loop_start, zone.loop_end)
    }

    // By hand: a loop of 882 frames at 44.1 kHz (441..=1322) is 882·96000/44100 = 1920 frames at
    // 96 kHz, 960..=2879; 4800 frames at 48 kHz (4800..=9599, to the file's last frame) are 4410
    // at 44.1 kHz, 4410..=8819, the converted file's last frame (ceil(9600·441/480) = 8820
    // frames); a region's `end`, inclusive too, scales the same way (frames 0..=9 at 22.05 kHz
    // are 0..=19 at 44.1 kHz); a one-frame loop at 96 kHz, shorter than a frame at 44.1 kHz,
    // keeps one frame.
    #[test]
    fn a_loops_end_scales_as_the_frame_after_it() {
        let up = "r.wav loop_mode=loop_continuous loop_start=441 loop_end=1322";
        let (_, _, start, end) = scaled(up, 4800, 44_100, 96_000);
        assert_eq!((start, end), (960, 2879));
        let down = "r.wav loop_mode=loop_continuous loop_start=4800 loop_end=9599";
        assert_eq!(scaled(down, 8820, 48_000, 44_100), (0, 8819, 4410, 8819));
        let end = "r.wav end=9 loop_mode=loop_continuous";
        assert_eq!(scaled(end, 40, 22_050, 44_100), (0, 19, 0, 19));
        let tiny = "r.wav loop_mode=loop_continuous loop_start=0 loop_end=0";
        assert_eq!(scaled(tiny, 40, 96_000, 44_100), (0, 39, 0, 0));
    }

    // The loop a conversion must hear: none for a region that does not loop; from its start
    // when it ends on the file's last frame (by its loop_end, or by default, or by an `end`
    // past the file); none when it ends before (the file's frames follow it) or is one the
    // zone refuses (past the region's end, or before its offset).
    #[test]
    fn a_loop_to_the_files_last_frame_goes_on_from_its_start() {
        let after = |sfz: &str| FileLoop::of(&region(sfz)).and_then(|l| l.after_last(100));
        assert_eq!(FileLoop::of(&region("r.wav")), None);
        assert_eq!(
            FileLoop::of(&region("r.wav loop_mode=one_shot loop_start=3")),
            None
        );
        assert_eq!(after("r.wav loop_start=40 loop_end=99"), Some(40));
        assert_eq!(after("r.wav loop_mode=loop_sustain loop_start=7"), Some(7));
        assert_eq!(after("r.wav loop_mode=loop_continuous"), Some(0));
        assert_eq!(after("r.wav loop_mode=loop_continuous end=500"), Some(0));
        assert_eq!(after("r.wav loop_start=40 loop_end=98"), None);
        assert_eq!(after("r.wav loop_mode=loop_continuous end=90"), None);
        assert_eq!(after("r.wav loop_start=40 loop_end=99 end=90"), None);
        assert_eq!(after("r.wav loop_start=40 loop_end=120"), None);
        assert_eq!(after("r.wav loop_start=99 loop_end=40"), None);
        assert_eq!(after("r.wav loop_mode=loop_continuous offset=99"), Some(0));
        assert_eq!(
            FileLoop::of(&region("r.wav loop_mode=loop_continuous")).and_then(|l| l.after_last(0)),
            None
        );
    }

    // By hand: 300 frames are 300 · 48000/44100 = 326.53 at 48 kHz and 300 · 44100/48000 =
    // 275.625 at 44.1 kHz, no whole number, so such a zone reads its file's own frames; 147
    // frames are 160 at 48 kHz, 882 are 1920 at 96 kHz and 4800 at 48 kHz are 4410 at 44.1 kHz,
    // whole, so they are converted; at the file's own rate nothing is; a loop to the default end
    // (the last of 1000 frames: 1088.4 at 48 kHz) counts as its length; a loop past the region's
    // end, which its zone plays without, and a region that loops not at all, are converted.
    #[test]
    fn a_loop_no_whole_frames_long_at_the_rate_reads_its_files_own_frames() {
        let own = |sfz: &str, frames: u64, from: u32, to: u32| {
            FileLoop::of(&region(sfz)).is_some_and(|l| l.reads_file_rate(frames, from, to))
        };
        let slip = "r.wav loop_start=2000 loop_end=2299";
        assert!(own(slip, 4000, 44_100, 48_000));
        assert!(own(slip, 4000, 48_000, 44_100));
        assert!(!own(slip, 4000, 44_100, 44_100));
        assert!(!own(
            "r.wav loop_start=0 loop_end=146",
            4000,
            44_100,
            48_000
        ));
        assert!(!own(
            "r.wav loop_start=441 loop_end=1322",
            4000,
            44_100,
            96_000
        ));
        assert!(!own(
            "r.wav loop_start=4800 loop_end=9599",
            9600,
            48_000,
            44_100
        ));
        assert!(own("r.wav loop_mode=loop_continuous", 1000, 44_100, 48_000));
        assert!(own(
            "r.wav loop_mode=loop_sustain loop_start=10",
            1000,
            44_100,
            48_000
        ));
        assert!(!own(
            "r.wav loop_start=2000 loop_end=4500",
            4000,
            44_100,
            48_000
        ));
        assert!(!own("r.wav end=100", 4000, 44_100, 48_000));
        assert!(!own(
            "r.wav loop_start=2000 loop_end=2299",
            0,
            44_100,
            48_000
        ));
    }

    // A zone on its file's own frames keeps its points as written (offset 3, frames 3..=3999,
    // loop 2000..=2299) and reads at 44100/48000 = 0.91875 times its speed (an octave up twice
    // that): at frame 10 position 3 + 9.1875, which a ramp's 4-point Hermite gives exactly (c1 =
    // 1, c2 = c3 = 0: 12 + 0.1875); its attack is in the project's frames (0.01 s at 48 kHz:
    // 480), 10/480 up at frame 10.
    #[test]
    fn a_zone_on_its_files_own_frames_reads_at_the_rates_ratio() {
        let ramp: Vec<f32> = (0..4000).map(|i| i as f32).collect();
        let sample = ZoneSample {
            buffer: SampleBuffer::from(ramp),
            file_rate: Some(44_100),
            at_file_rate: true,
        };
        let slip = region("r.wav loop_start=2000 loop_end=2299 ampeg_attack=0.01 offset=3");
        let (instrument, problems) = Instrument::build(&[slip], |_| sample.clone(), 48_000);
        assert_eq!(problems, Vec::<String>::new());
        let zone = &instrument.zones[0];
        assert_eq!(
            (
                zone.first,
                zone.last,
                zone.loop_start,
                zone.loop_end,
                zone.attack
            ),
            (3, 3999, 2000, 2299, 480)
        );
        assert_eq!(zone.speed(60), 0.918_75);
        assert_eq!(zone.speed(72), 2.0 * 0.918_75);
        let level = 10.0 / 480.0;
        let (value, envelope) = zone.frame(10, None, zone.speed(60)).expect("sounds");
        assert_eq!((value[0], envelope), (3.0 + 9.1875, level as f32));
    }

    // A loop that runs past the sound plays without one, said once; a silent sound and an
    // offset past the end leave the region out.
    #[test]
    fn regions_that_cannot_loop_or_play() {
        let looped = region("a.wav loop_start=2 loop_end=40");
        let (instrument, problems) =
            Instrument::build(&[looped], |_| loaded(vec![1.0; 10]), 44_100);
        assert_eq!(instrument.zones[0].mode, LoopMode::NoLoop);
        assert_eq!(
            problems,
            ["t.sfz:1: loop 2..40 is outside frames 0..9; it plays without a loop"]
        );
        let late = region("a.wav offset=12");
        let (instrument, problems) = Instrument::build(&[late], |_| loaded(vec![1.0; 10]), 44_100);
        assert_eq!(instrument.zones(), 0);
        assert_eq!(
            problems,
            ["t.sfz:1: offset 12 is past the sound's last frame; the region is left out"]
        );
        let (instrument, problems) =
            Instrument::build(&[region("gone.wav")], |_| loaded(Vec::new()), 44_100);
        assert_eq!(instrument.zones(), 0);
        assert_eq!(
            problems,
            ["t.sfz:1: gone.wav has no sound; the region is left out"]
        );
    }

    // A continuous loop of frames 2..=4 read at speed 1: 0 1 2 3 4 2 3 4 2 ...; between frames
    // the 4-point Hermite half way, (-y0 + 9 y1 + 9 y2 - y3)/16, its points running through the
    // loop: past the loop's end its start, and once wrapped, before its start its end.
    #[test]
    fn a_continuous_loop_wraps_and_reads_its_start_after_its_end() {
        let ramp: Vec<f32> = (0..8).map(|i| i as f32).collect();
        let zone = built(
            &region("r.wav loop_mode=loop_continuous loop_start=2 loop_end=4"),
            ramp,
        );
        let read: Vec<f32> = (0..9)
            .map(|t| zone.frame(t, None, 1.0).expect("sounds").0[0])
            .collect();
        assert_eq!(read, [0.0, 1.0, 2.0, 3.0, 4.0, 2.0, 3.0, 4.0, 2.0]);
        // At speed 0.5, frame 9 reads position 4.5, not yet wrapped: the points 3, 4, then the
        // loop's start 2 and 3, (-3 + 36 + 18 - 3)/16 = 3. Frame 11 reads 5.5, wrapped to 2.5:
        // the points are the loop's end 4, then 2, 3, 4: (-4 + 18 + 27 - 4)/16 = 2.3125 (frame 1
        // before 2, the voice's way in, would give the straight line's 2.5).
        assert_eq!(zone.frame(9, None, 0.5).expect("sounds").0[0], 3.0);
        assert_eq!(zone.frame(11, None, 0.5).expect("sounds").0[0], 2.3125);
    }

    // Released at frame 4 over 8 frames: level 2^(−15 r/8), then nothing; with no release the
    // voice ends at note-off; a one-shot zone plays on through note-off to its end.
    #[test]
    fn the_release_falls_in_decibels_and_ends() {
        let zone = built(
            &region("r.wav ampeg_release=0.000181405895691610"),
            vec![1.0; 100],
        );
        assert_eq!(zone.release, 8);
        assert_eq!(zone.frame(4, Some(4), 1.0).expect("sounds").1, 1.0);
        let level = zone.frame(6, Some(4), 1.0).expect("sounds").1;
        assert_eq!(level, exp2(-(15.0 * 2.0 / 8.0)) as f32);
        assert!(zone.frame(12, Some(4), 1.0).is_none());
        let cut = built(&region("r.wav"), vec![1.0; 100]);
        assert!(cut.frame(3, Some(4), 1.0).is_some());
        assert!(cut.frame(4, Some(4), 1.0).is_none());
        let shot = built(&region("r.wav loop_mode=one_shot"), vec![1.0; 100]);
        assert!(shot.frame(99, Some(4), 1.0).is_some());
        assert!(shot.frame(100, Some(4), 1.0).is_none());
    }

    // A round robin of two: notes 1, 2, 3 sound positions 1, 2, 1.
    #[test]
    fn a_round_robin_takes_turns() {
        let first = region("a.wav seq_length=2 seq_position=1");
        let second = region("b.wav seq_length=2 seq_position=2");
        let (instrument, _) = Instrument::build(&[first, second], |_| loaded(vec![1.0]), 44_100);
        let counters: Vec<AtomicU32> = (0..instrument.counters())
            .map(|_| AtomicU32::new(0))
            .collect();
        let mut played = Vec::new();
        for _ in 0..3 {
            instrument.start_shaped(60, 1.0, &counters, None, |zone, _, _| played.push(zone));
        }
        assert_eq!(played, [0, 1, 0]);
    }
}
